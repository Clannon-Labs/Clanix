use std::{
    io,
    process::{Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    time::{Instant, timeout, timeout_at},
};

use crate::{
    error::{RuntimeError, RuntimeErrorKind},
    terminal::TerminalDimensions,
};

const TERMINAL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const TERMINAL_REAP_TIMEOUT: Duration = Duration::from_secs(1);
const ROOTLESS_POLICY: CommandPolicy = CommandPolicy::new(
    Duration::from_secs(10),
    4 * 1024,
    64 * 1024,
    64 * 1024,
    "Podman rootless verification timed out",
);
const CREATE_POLICY: CommandPolicy = CommandPolicy::new(
    Duration::from_secs(300),
    64 * 1024,
    256 * 1024,
    256 * 1024,
    "Podman container creation timed out",
);
const INSPECT_POLICY: CommandPolicy = CommandPolicy::new(
    Duration::from_secs(10),
    1024 * 1024,
    64 * 1024,
    1024 * 1024,
    "environment inspection timed out",
);
const REMOVE_POLICY: CommandPolicy = CommandPolicy::new(
    Duration::from_secs(10),
    64 * 1024,
    64 * 1024,
    64 * 1024,
    "Podman container removal timed out",
);
const MARKER_POLICY: CommandPolicy = CommandPolicy::new(
    TERMINAL_HANDSHAKE_TIMEOUT,
    4 * 1024,
    64 * 1024,
    64 * 1024,
    "terminal PTY marker handshake timed out",
);
const MARKER_REMOVE_POLICY: CommandPolicy = CommandPolicy::new(
    TERMINAL_HANDSHAKE_TIMEOUT,
    4 * 1024,
    64 * 1024,
    64 * 1024,
    "terminal PTY marker cleanup timed out",
);
const RESIZE_POLICY: CommandPolicy = CommandPolicy::new(
    Duration::from_secs(2),
    4 * 1024,
    64 * 1024,
    64 * 1024,
    "terminal resize timed out",
);
const TERMINAL_WRAPPER: &str = "umask 077; test -t 0 && test -t 1 && test -t 2 || exit 64; tty_path=$(tty) || exit; case \"$tty_path\" in /dev/pts/*) tty_number=${tty_path#/dev/pts/}; case \"$tty_number\" in ''|*[!0-9]*) exit 64;; esac;; *) exit 64;; esac; stty rows \"$2\" cols \"$3\" < \"$tty_path\" || exit; printf '%s\\n' \"$tty_path\" > \"$1\" || exit; exec /bin/sh";
const MARKER_READER: &str = "attempt=0; while [ \"$attempt\" -lt 250 ]; do if test -s \"$1\"; then cat \"$1\"; exit; fi; attempt=$((attempt + 1)); sleep 0.01; done; exit 75";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommandPolicy {
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    output_limit: usize,
    timeout_message: &'static str,
}

impl CommandPolicy {
    const fn new(
        timeout: Duration,
        stdout_limit: usize,
        stderr_limit: usize,
        output_limit: usize,
        timeout_message: &'static str,
    ) -> Self {
        Self {
            timeout,
            stdout_limit,
            stderr_limit,
            output_limit,
            timeout_message,
        }
    }

    fn deadline(self) -> Instant {
        Instant::now() + self.timeout
    }
}

pub(crate) struct OpenedTerminal {
    pub(crate) child: Child,
    pub(crate) pty_path: String,
}

pub(crate) async fn verify_rootless() -> Result<(), String> {
    let output = bounded_podman_command(
        ["info", "--format", "{{.Host.Security.Rootless}}"],
        ROOTLESS_POLICY,
    )
    .await
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    if String::from_utf8_lossy(&output.stdout).trim() != "true" {
        return Err("Podman is not running rootless".to_owned());
    }
    Ok(())
}

pub(crate) async fn create_container(name: &str, image: &str) -> Result<(), RuntimeError> {
    let output = bounded_podman_command(create_args(name, image), CREATE_POLICY)
        .await
        .map_err(|error| RuntimeError::internal("could not start Podman", error))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            "could not create environment",
            &output.stderr,
        ))
    }
}

pub(crate) async fn open_terminal(
    container: &str,
    generation: u64,
    dimensions: TerminalDimensions,
) -> io::Result<OpenedTerminal> {
    let marker = terminal_marker(generation);
    let mut child = Command::new("podman")
        .args(terminal_args(container, &marker, dimensions))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let deadline = Instant::now() + TERMINAL_HANDSHAKE_TIMEOUT;

    let marker_bytes = match read_terminal_marker(container, &marker, deadline).await {
        Ok(marker_bytes) => marker_bytes,
        Err(error) => {
            clean_failed_terminal(container, &marker, child).await;
            return Err(error);
        }
    };
    let pty_path = match parse_pty_path(&marker_bytes) {
        Ok(pty_path) => pty_path,
        Err(error) => {
            clean_failed_terminal(container, &marker, child).await;
            return Err(error);
        }
    };

    match remove_terminal_marker(container, &marker, deadline).await {
        Ok(()) => {}
        Err(error) => {
            clean_failed_terminal(container, &marker, child).await;
            return Err(error);
        }
    }

    if let Some(status) = child.try_wait()? {
        return Err(io::Error::other(format!(
            "terminal process exited during PTY handshake with status {status}"
        )));
    }

    Ok(OpenedTerminal { child, pty_path })
}

pub(crate) async fn resize_terminal(
    container: &str,
    pty_path: &str,
    dimensions: TerminalDimensions,
) -> io::Result<()> {
    debug_assert!(parse_pty_path(format!("{pty_path}\n").as_bytes()).is_ok());
    let output = bounded_terminal_command(
        resize_args(container, pty_path, dimensions),
        RESIZE_POLICY.deadline(),
        RESIZE_POLICY,
    )
    .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io_command_error(
            "could not resize terminal",
            &output.stderr,
        ))
    }
}

fn terminal_marker(generation: u64) -> String {
    format!("/tmp/.clannon-terminal-{generation:016x}.tty")
}

fn create_args(name: &str, image: &str) -> Vec<String> {
    [
        "run",
        "--detach",
        "--rm",
        "--name",
        name,
        "--network=none",
        "--cap-drop=all",
        "--security-opt=no-new-privileges",
        "--pids-limit=256",
        "--memory=512m",
        "--cpus=1",
        "--tmpfs",
        "/workspace:rw,exec,nosuid,size=256m",
        "--workdir=/workspace",
        image,
        "/bin/sh",
        "-c",
        "trap 'exit 0' TERM INT; while :; do sleep 3600; done",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn terminal_args(container: &str, marker: &str, dimensions: TerminalDimensions) -> Vec<String> {
    vec![
        "exec".into(),
        "-i".into(),
        "-t".into(),
        "--detach-keys=".into(),
        "--workdir".into(),
        "/workspace".into(),
        "--env".into(),
        "TERM=dumb".into(),
        container.into(),
        "/bin/sh".into(),
        "-c".into(),
        TERMINAL_WRAPPER.into(),
        "clannon-terminal".into(),
        marker.into(),
        dimensions.rows().to_string(),
        dimensions.columns().to_string(),
    ]
}

fn marker_reader_args(container: &str, marker: &str) -> Vec<String> {
    vec![
        "exec".into(),
        container.into(),
        "/bin/sh".into(),
        "-c".into(),
        MARKER_READER.into(),
        "clannon-read-terminal-marker".into(),
        marker.into(),
    ]
}

fn remove_marker_args(container: &str, marker: &str) -> Vec<String> {
    vec![
        "exec".into(),
        container.into(),
        "rm".into(),
        "-f".into(),
        marker.into(),
    ]
}

fn resize_args(container: &str, pty_path: &str, dimensions: TerminalDimensions) -> Vec<String> {
    vec![
        "exec".into(),
        container.into(),
        "stty".into(),
        "-F".into(),
        pty_path.into(),
        "rows".into(),
        dimensions.rows().to_string(),
        "cols".into(),
        dimensions.columns().to_string(),
    ]
}

async fn read_terminal_marker(
    container: &str,
    marker: &str,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let output = bounded_terminal_command(
        marker_reader_args(container, marker),
        deadline,
        MARKER_POLICY,
    )
    .await?;
    if output.status.success() {
        Ok(output.stdout)
    } else if output.status.code() == Some(75) {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "terminal PTY marker was not published",
        ))
    } else {
        Err(io_command_error(
            "could not discover terminal PTY marker",
            &output.stderr,
        ))
    }
}

async fn remove_terminal_marker(
    container: &str,
    marker: &str,
    deadline: Instant,
) -> io::Result<()> {
    let output = bounded_terminal_command(
        remove_marker_args(container, marker),
        deadline,
        MARKER_REMOVE_POLICY,
    )
    .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io_command_error(
            "could not remove terminal PTY marker",
            &output.stderr,
        ))
    }
}

fn parse_pty_path(marker: &[u8]) -> io::Result<String> {
    let Some(path) = marker.strip_suffix(b"\n") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "terminal PTY marker did not end with one newline",
        ));
    };
    let Some(number) = path.strip_prefix(b"/dev/pts/") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "terminal PTY marker contained an invalid path",
        ));
    };
    if number.is_empty() || !number.iter().all(u8::is_ascii_digit) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "terminal PTY marker contained an invalid path",
        ));
    }
    String::from_utf8(path.to_vec()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "terminal PTY marker was not ASCII",
        )
    })
}

async fn clean_failed_terminal(container: &str, marker: &str, child: Child) {
    let _ = remove_terminal_marker(container, marker, Instant::now() + TERMINAL_REAP_TIMEOUT).await;
    kill_and_reap(child).await;
}

async fn bounded_terminal_command(
    arguments: Vec<String>,
    deadline: Instant,
    policy: CommandPolicy,
) -> io::Result<Output> {
    bounded_command_until("podman", arguments, deadline, policy).await
}

async fn bounded_command(
    program: &str,
    arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    policy: CommandPolicy,
) -> io::Result<Output> {
    bounded_command_until(program, arguments, policy.deadline(), policy).await
}

async fn bounded_podman_command(
    arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    policy: CommandPolicy,
) -> io::Result<Output> {
    bounded_command("podman", arguments, policy).await
}

async fn bounded_command_until(
    program: &str,
    arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    deadline: Instant,
    policy: CommandPolicy,
) -> io::Result<Output> {
    if deadline <= Instant::now() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            policy.timeout_message,
        ));
    }

    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("terminal command stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("terminal command stderr was not piped"))?;
    let output_used = Arc::new(AtomicUsize::new(0));

    let completed = timeout_at(deadline, async {
        tokio::try_join!(
            child.wait(),
            read_bounded(
                stdout,
                policy.stdout_limit,
                "stdout",
                policy.output_limit,
                output_used.clone()
            ),
            read_bounded(
                stderr,
                policy.stderr_limit,
                "stderr",
                policy.output_limit,
                output_used
            )
        )
    })
    .await;
    match completed {
        Ok(Ok((status, stdout, stderr))) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        Ok(Err(error)) => {
            kill_and_reap(child).await;
            Err(error)
        }
        Err(_) => {
            kill_and_reap(child).await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                policy.timeout_message,
            ))
        }
    }
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
    stream: &'static str,
    output_limit: usize,
    output_used: Arc<AtomicUsize>,
) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("command {stream} exceeded the {limit}-byte limit"),
            ));
        }
        if output_used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(count).filter(|next| *next <= output_limit)
            })
            .is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("command output exceeded the aggregate {output_limit}-byte limit"),
            ));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

async fn kill_and_reap(mut child: Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.start_kill();
    if timeout(TERMINAL_REAP_TIMEOUT, child.wait()).await.is_err() {
        let _ = child.start_kill();
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
    }
}

fn io_command_error(context: &str, stderr: &[u8]) -> io::Error {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    io::Error::other(if detail.is_empty() {
        context.to_owned()
    } else {
        format!("{context}: {detail}")
    })
}

pub(crate) async fn exec(container: &str, script: &str) -> Result<String, RuntimeError> {
    let output =
        bounded_podman_command(["exec", container, "/bin/sh", "-c", script], INSPECT_POLICY)
            .await
            .map_err(|error| RuntimeError::internal("could not inspect environment", error))?;
    if !output.status.success() {
        return Err(command_error(
            "environment inspection failed",
            &output.stderr,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) async fn remove_container(name: &str) -> Result<(), RuntimeError> {
    let output = bounded_podman_command(
        ["rm", "--force", "--ignore", "--time=1", name],
        REMOVE_POLICY,
    )
    .await
    .map_err(|error| RuntimeError::internal("could not destroy environment", error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            "could not destroy environment",
            &output.stderr,
        ))
    }
}

fn command_error(context: &str, stderr: &[u8]) -> RuntimeError {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    RuntimeError::new(
        RuntimeErrorKind::BadGateway,
        if detail.is_empty() {
            context.to_owned()
        } else {
            format!("{context}: {detail}")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimensions(columns: u16, rows: u16) -> TerminalDimensions {
        TerminalDimensions::new(columns, rows).unwrap()
    }

    #[test]
    fn container_argv_disables_guest_network_without_weakening_existing_limits() {
        let args = create_args("clannon-test", "example.invalid/image:test");

        assert_eq!(args[0], "run");
        assert_eq!(
            args.iter()
                .filter(|argument| *argument == "--network=none")
                .count(),
            1
        );
        assert!(args.iter().any(|argument| argument == "--cap-drop=all"));
        assert!(
            args.iter()
                .any(|argument| argument == "--security-opt=no-new-privileges")
        );
        assert!(args.iter().any(|argument| argument == "--pids-limit=256"));
        assert!(args.iter().any(|argument| argument == "--memory=512m"));
        assert!(args.iter().any(|argument| argument == "--cpus=1"));
        assert!(
            args.iter()
                .position(|argument| argument == "--network=none")
                .unwrap()
                < args
                    .iter()
                    .position(|argument| argument == "example.invalid/image:test")
                    .unwrap()
        );
    }

    #[test]
    fn one_shot_policies_have_deadlines_and_stream_caps() {
        for policy in [
            ROOTLESS_POLICY,
            CREATE_POLICY,
            INSPECT_POLICY,
            REMOVE_POLICY,
            MARKER_POLICY,
            MARKER_REMOVE_POLICY,
            RESIZE_POLICY,
        ] {
            assert!(!policy.timeout.is_zero());
            assert!(policy.stdout_limit > 0);
            assert!(policy.stderr_limit > 0);
            assert!(policy.output_limit > 0);
            assert!(!policy.timeout_message.is_empty());
        }
    }

    #[test]
    fn terminal_argv_allocates_a_tty_and_initializes_it_before_publication() {
        let marker = terminal_marker(42);
        let args = terminal_args("clannon-test", &marker, dimensions(132, 43));

        assert_eq!(
            &args[..9],
            [
                "exec",
                "-i",
                "-t",
                "--detach-keys=",
                "--workdir",
                "/workspace",
                "--env",
                "TERM=dumb",
                "clannon-test",
            ]
        );
        assert_eq!(args[9], "/bin/sh");
        assert_eq!(args[10], "-c");
        assert!(args[11].contains("test -t 0 && test -t 1 && test -t 2"));
        assert!(args[11].contains("*[!0-9]*"));
        assert!(
            args[11].find("stty rows").unwrap() < args[11].find("printf '%s\\n'").unwrap(),
            "initial stty must happen before marker publication"
        );
        assert_eq!(args[12], "clannon-terminal");
        assert_eq!(args[13], marker);
        assert_eq!(&args[14..], ["43", "132"]);
    }

    #[test]
    fn marker_commands_are_generation_private_and_separate_from_live_output() {
        let first = terminal_marker(1);
        let second = terminal_marker(2);
        assert_ne!(first, second);
        assert!(first.starts_with("/tmp/.clannon-terminal-"));

        let reader = marker_reader_args("clannon-test", &first);
        assert_eq!(reader[0], "exec");
        assert_eq!(reader[1], "clannon-test");
        assert_eq!(reader.last(), Some(&first));
        let remover = remove_marker_args("clannon-test", &first);
        assert_eq!(
            remover,
            ["exec", "clannon-test", "rm", "-f", first.as_str()]
        );

        let live = terminal_args("clannon-test", &first, dimensions(80, 24));
        assert!(live.iter().all(|argument| argument != MARKER_READER));
    }

    #[test]
    fn pty_path_parser_is_strict_and_requires_a_complete_marker() {
        assert_eq!(parse_pty_path(b"/dev/pts/0\n").unwrap(), "/dev/pts/0");
        assert_eq!(
            parse_pty_path(b"/dev/pts/987654321\n").unwrap(),
            "/dev/pts/987654321"
        );

        for invalid in [
            b"".as_slice(),
            b"/dev/pts/7".as_slice(),
            b"/dev/pts/\n".as_slice(),
            b"/dev/pts/7\n/dev/pts/8\n".as_slice(),
            b"/dev/pts/7x\n".as_slice(),
            b" /dev/pts/7\n".as_slice(),
            b"/dev/tty7\n".as_slice(),
            b"/dev/pts/7\r\n".as_slice(),
            b"/dev/pts/\xff\n".as_slice(),
        ] {
            assert!(parse_pty_path(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn resize_argv_uses_validated_path_without_a_shell() {
        assert_eq!(
            resize_args("clannon-test", "/dev/pts/12", dimensions(151, 47)),
            [
                "exec",
                "clannon-test",
                "stty",
                "-F",
                "/dev/pts/12",
                "rows",
                "47",
                "cols",
                "151",
            ]
        );
    }

    #[tokio::test]
    async fn command_output_enforces_stream_and_aggregate_caps() {
        let error = read_bounded(
            tokio::io::repeat(0),
            64 * 1024,
            "stdout",
            64 * 1024,
            Arc::new(AtomicUsize::new(0)),
        )
        .await
        .expect_err("an endless command stream must exceed the bound");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "command stdout exceeded the 65536-byte limit"
        );

        let stdout_policy =
            CommandPolicy::new(Duration::from_secs(1), 3, 8, 8, "test command timed out");
        let stdout_error = bounded_command("/bin/sh", ["-c", "printf 1234"], stdout_policy)
            .await
            .expect_err("stdout must be capped independently");
        assert_eq!(stdout_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            stdout_error.to_string(),
            "command stdout exceeded the 3-byte limit"
        );

        let stderr_policy =
            CommandPolicy::new(Duration::from_secs(1), 8, 3, 8, "test command timed out");
        let stderr_error = bounded_command("/bin/sh", ["-c", "printf 1234 >&2"], stderr_policy)
            .await
            .expect_err("stderr must be capped independently");
        assert_eq!(stderr_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            stderr_error.to_string(),
            "command stderr exceeded the 3-byte limit"
        );

        let aggregate_policy =
            CommandPolicy::new(Duration::from_secs(1), 4, 4, 5, "test command timed out");
        let aggregate_error = bounded_command(
            "/bin/sh",
            ["-c", "printf 123; printf 456 >&2"],
            aggregate_policy,
        )
        .await
        .expect_err("combined stdout and stderr must share one aggregate cap");
        assert_eq!(aggregate_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            aggregate_error.to_string(),
            "command output exceeded the aggregate 5-byte limit"
        );
    }

    #[tokio::test]
    async fn command_deadline_kills_and_reaps_the_child() {
        let started = Instant::now();
        let policy = CommandPolicy::new(
            Duration::from_millis(20),
            64,
            64,
            64,
            "test command timed out",
        );
        let error = bounded_command("/bin/sleep", ["30"], policy)
            .await
            .expect_err("sleep must be stopped at the deadline");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "test command timed out");
        assert!(
            started.elapsed() < TERMINAL_REAP_TIMEOUT * 2,
            "deadline handling and reaping must remain bounded"
        );
    }
}
