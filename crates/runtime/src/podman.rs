use std::{
    io,
    process::{Output, Stdio},
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
const TERMINAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const TERMINAL_REAP_TIMEOUT: Duration = Duration::from_secs(1);
const TERMINAL_COMMAND_OUTPUT_LIMIT: usize = 64 * 1024;
const TERMINAL_WRAPPER: &str = "umask 077; test -t 0 && test -t 1 && test -t 2 || exit 64; tty_path=$(tty) || exit; case \"$tty_path\" in /dev/pts/*) tty_number=${tty_path#/dev/pts/}; case \"$tty_number\" in ''|*[!0-9]*) exit 64;; esac;; *) exit 64;; esac; stty rows \"$2\" cols \"$3\" < \"$tty_path\" || exit; printf '%s\\n' \"$tty_path\" > \"$1\" || exit; exec /bin/sh";
const MARKER_READER: &str = "attempt=0; while [ \"$attempt\" -lt 250 ]; do if test -s \"$1\"; then cat \"$1\"; exit; fi; attempt=$((attempt + 1)); sleep 0.01; done; exit 75";

pub(crate) struct OpenedTerminal {
    pub(crate) child: Child,
    pub(crate) pty_path: String,
}

pub(crate) async fn verify_rootless() -> Result<(), String> {
    let output = Command::new("podman")
        .args(["info", "--format", "{{.Host.Security.Rootless}}"])
        .output()
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
    let output = Command::new("podman")
        .args([
            "run",
            "--detach",
            "--rm",
            "--name",
            name,
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
        ])
        .output()
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
        Instant::now() + TERMINAL_COMMAND_TIMEOUT,
        "terminal resize timed out",
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
        "terminal PTY marker handshake timed out",
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
        "terminal PTY marker cleanup timed out",
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
    timeout_message: &'static str,
) -> io::Result<Output> {
    bounded_command("podman", arguments, deadline, timeout_message).await
}

async fn bounded_command(
    program: &str,
    arguments: Vec<String>,
    deadline: Instant,
    timeout_message: &'static str,
) -> io::Result<Output> {
    if deadline <= Instant::now() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
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

    let completed = timeout_at(deadline, async {
        tokio::try_join!(child.wait(), read_bounded(stdout), read_bounded(stderr))
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
            Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message))
        }
    }
}

async fn read_bounded(mut reader: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > TERMINAL_COMMAND_OUTPUT_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "terminal command output exceeded the {TERMINAL_COMMAND_OUTPUT_LIMIT}-byte limit"
                ),
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
    let output = Command::new("podman")
        .args(["exec", container, "/bin/sh", "-c", script])
        .output()
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
    let output = Command::new("podman")
        .args(["rm", "--force", "--ignore", "--time=1", name])
        .output()
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
    async fn terminal_command_output_is_bounded() {
        let error = read_bounded(tokio::io::repeat(0))
            .await
            .expect_err("an endless command stream must exceed the bound");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("65536-byte limit"));
    }

    #[tokio::test]
    async fn terminal_command_deadline_kills_and_reaps_the_child() {
        let started = Instant::now();
        let error = bounded_command(
            "/bin/sleep",
            vec!["30".into()],
            Instant::now() + Duration::from_millis(20),
            "test command timed out",
        )
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
