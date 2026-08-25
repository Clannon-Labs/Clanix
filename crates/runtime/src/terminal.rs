use std::{
    collections::HashMap,
    fmt,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::Child,
    sync::{Mutex, Notify, broadcast, mpsc},
    time::{Duration, Instant, timeout, timeout_at},
};

use crate::{
    environment::{Environment, Transcript},
    error::{RuntimeError, RuntimeErrorKind},
    podman,
};

const EVENT_CAPACITY: usize = 32;
const PUMP_CAPACITY: usize = 32;
const PROCESS_DRAIN_GRACE: Duration = Duration::from_millis(500);
const PROCESS_FORCE_GRACE: Duration = Duration::from_secs(1);
const READER_DRAIN_GRACE: Duration = Duration::from_millis(250);

type BoxReader = Box<dyn AsyncRead + Unpin + Send>;
type BoxWriter = Box<dyn AsyncWrite + Unpin + Send>;
type ProcessWait = Pin<Box<dyn Future<Output = ProcessEnd> + Send>>;
type WaitProcess = Box<dyn FnOnce(mpsc::Receiver<String>) -> ProcessWait + Send>;

pub enum TerminalInput {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Output(Vec<u8>),
    Exited { code: Option<i32> },
    Failed(String),
}

pub struct TerminalOutput(Vec<u8>);

impl TerminalOutput {
    pub fn into_text(self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

pub struct TerminalReservation {
    environment: Option<Arc<Environment>>,
}

pub struct TerminalAttachment {
    environment: Option<Arc<Environment>>,
    generation: u64,
    stdin: Arc<Mutex<BoxWriter>>,
    events: broadcast::Receiver<TerminalEvent>,
}

#[derive(Debug)]
pub struct TerminalOpenError(io::Error);

impl fmt::Display for TerminalOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for TerminalOpenError {}

pub(crate) struct TerminalHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    transcript: Arc<Transcript>,
    state: std::sync::Mutex<HubState>,
    process_finished: Notify,
}

struct HubState {
    next_generation: u64,
    active: Option<ActiveTerminal>,
    processes: HashMap<u64, mpsc::Sender<String>>,
}

struct ActiveTerminal {
    generation: u64,
    accepting: Arc<AtomicBool>,
    stdin: Arc<Mutex<BoxWriter>>,
    events: broadcast::Sender<TerminalEvent>,
}

struct AttachmentParts {
    generation: u64,
    stdin: Arc<Mutex<BoxWriter>>,
    events: broadcast::Receiver<TerminalEvent>,
}

struct TerminalProcess {
    stdin: BoxWriter,
    stdout: BoxReader,
    stderr: BoxReader,
    wait: WaitProcess,
}

struct SupervisedProcess {
    stdout: BoxReader,
    stderr: BoxReader,
    wait: WaitProcess,
    stop_receiver: mpsc::Receiver<String>,
}

struct Supervisor {
    hub: Weak<HubInner>,
    generation: u64,
    accepting: Arc<AtomicBool>,
    events: broadcast::Sender<TerminalEvent>,
}

enum PumpMessage {
    Output(Vec<u8>),
    ReaderFailed(String),
    ReaderDone,
    ProcessEnded(ProcessEnd),
}

enum ProcessEnd {
    Exited(Option<i32>),
    Failed(String),
}

pub(crate) fn reserve(environment: Arc<Environment>) -> Result<TerminalReservation, RuntimeError> {
    if !environment.try_open_terminal() {
        return Err(RuntimeError::new(
            RuntimeErrorKind::Conflict,
            "a terminal is already connected",
        ));
    }
    Ok(TerminalReservation {
        environment: Some(environment),
    })
}

impl TerminalReservation {
    pub fn open(mut self) -> Result<TerminalAttachment, TerminalOpenError> {
        self.open_with(|container| {
            podman::spawn_terminal(container).and_then(TerminalProcess::from_child)
        })
    }

    fn open_with(
        &mut self,
        spawn: impl FnOnce(&str) -> io::Result<TerminalProcess>,
    ) -> Result<TerminalAttachment, TerminalOpenError> {
        let environment = self
            .environment
            .take()
            .expect("terminal reservation owns its environment");
        let parts = match environment
            .terminal()
            .attach(environment.container_name(), spawn)
        {
            Ok(parts) => parts,
            Err(error) => {
                environment.close_terminal();
                return Err(TerminalOpenError(error));
            }
        };

        Ok(TerminalAttachment {
            environment: Some(environment),
            generation: parts.generation,
            stdin: parts.stdin,
            events: parts.events,
        })
    }
}

impl Drop for TerminalReservation {
    fn drop(&mut self) {
        if let Some(environment) = self.environment.take() {
            environment.close_terminal();
        }
    }
}

impl TerminalAttachment {
    pub async fn send(&mut self, input: TerminalInput) -> io::Result<()> {
        let environment = self
            .environment
            .as_ref()
            .expect("open terminal attachment owns its environment");
        let (transcript, bytes) = match input {
            TerminalInput::Text(text) => (text.clone(), text.into_bytes()),
            TerminalInput::Binary(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), bytes),
        };

        let result = self.stdin.lock().await.write_all(&bytes).await;
        match &result {
            Ok(()) => environment.record_transcript("input", transcript).await,
            Err(error) => environment.terminal().request_failure(
                self.generation,
                format!("could not write terminal input: {error}"),
            ),
        }
        result
    }

    pub async fn next_event(&mut self) -> Option<TerminalEvent> {
        match self.events.recv().await {
            Ok(event) => Some(event),
            Err(broadcast::error::RecvError::Lagged(count)) => Some(TerminalEvent::Failed(
                format!("terminal attachment lagged and missed {count} events"),
            )),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }

    pub async fn next_output(&mut self) -> Option<TerminalOutput> {
        match self.next_event().await? {
            TerminalEvent::Output(bytes) => Some(TerminalOutput(bytes)),
            TerminalEvent::Exited { .. } | TerminalEvent::Failed(_) => None,
        }
    }

    pub async fn close(mut self) {
        self.release();
    }

    fn release(&mut self) {
        if let Some(environment) = self.environment.take() {
            environment.close_terminal();
        }
    }
}

impl Drop for TerminalAttachment {
    fn drop(&mut self) {
        self.release();
    }
}

impl TerminalHub {
    pub(crate) fn new(transcript: Arc<Transcript>) -> Self {
        Self {
            inner: Arc::new(HubInner {
                transcript,
                state: std::sync::Mutex::new(HubState {
                    next_generation: 1,
                    active: None,
                    processes: HashMap::new(),
                }),
                process_finished: Notify::new(),
            }),
        }
    }

    fn attach(
        &self,
        container: &str,
        spawn: impl FnOnce(&str) -> io::Result<TerminalProcess>,
    ) -> io::Result<AttachmentParts> {
        let mut state = self.inner.state.lock().expect("terminal hub lock poisoned");
        if let Some(active) = &state.active
            && active.accepting.load(Ordering::Acquire)
        {
            return Ok(AttachmentParts {
                generation: active.generation,
                stdin: active.stdin.clone(),
                events: active.events.subscribe(),
            });
        }

        let handle = tokio::runtime::Handle::try_current().map_err(|error| {
            io::Error::other(format!(
                "terminal must open inside a Tokio runtime: {error}"
            ))
        })?;
        let process = spawn(container)?;
        let TerminalProcess {
            stdin: process_stdin,
            stdout,
            stderr,
            wait,
        } = process;
        let generation = state.next_generation;
        state.next_generation += 1;
        let accepting = Arc::new(AtomicBool::new(true));
        let stdin = Arc::new(Mutex::new(process_stdin));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let event_receiver = events.subscribe();
        let (stop, stop_receiver) = mpsc::channel(1);
        state.processes.insert(generation, stop);

        state.active = Some(ActiveTerminal {
            generation,
            accepting: accepting.clone(),
            stdin: stdin.clone(),
            events: events.clone(),
        });
        handle.spawn(supervise(
            Supervisor {
                hub: Arc::downgrade(&self.inner),
                generation,
                accepting,
                events,
            },
            SupervisedProcess {
                stdout,
                stderr,
                wait,
                stop_receiver,
            },
        ));

        Ok(AttachmentParts {
            generation,
            stdin,
            events: event_receiver,
        })
    }

    fn request_failure(&self, generation: u64, message: String) {
        self.inner.request_stop(generation, message);
    }

    pub(crate) async fn finish_after_container_removed(&self) {
        let _ = self
            .finish_with_timeouts(PROCESS_DRAIN_GRACE, PROCESS_FORCE_GRACE)
            .await;
    }

    pub(crate) async fn stop_and_reap(&self) {
        self.inner
            .stop_all("terminal container cleanup failed; stopping proxy");
        let _ = timeout(PROCESS_FORCE_GRACE, self.wait_for_processes()).await;
    }

    async fn finish_with_timeouts(&self, drain: Duration, force: Duration) -> bool {
        if timeout(drain, self.wait_for_processes()).await.is_ok() {
            return true;
        }
        self.inner
            .stop_all("terminal container is gone; stopping lingering proxy");
        timeout(force, self.wait_for_processes()).await.is_ok()
    }

    async fn wait_for_processes(&self) {
        loop {
            let process_finished = self.inner.process_finished.notified();
            if self
                .inner
                .state
                .lock()
                .expect("terminal hub lock poisoned")
                .processes
                .is_empty()
            {
                return;
            }
            process_finished.await;
        }
    }
}

impl HubInner {
    fn request_stop(&self, generation: u64, message: String) {
        let state = self.state.lock().expect("terminal hub lock poisoned");
        if let Some(stop) = state.processes.get(&generation) {
            let _ = stop.try_send(message);
        }
    }

    fn stop_all(&self, message: &str) {
        let state = self.state.lock().expect("terminal hub lock poisoned");
        for stop in state.processes.values() {
            let _ = stop.try_send(message.to_owned());
        }
    }
}

impl TerminalProcess {
    fn from_child(mut child: Child) -> io::Result<Self> {
        let handles = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(stdin), Some(stdout), Some(stderr)) = handles else {
            let _ = child.start_kill();
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            return Err(io::Error::other("terminal process handles were not piped"));
        };
        let wait = Box::new(move |stop| Box::pin(wait_for_child(child, stop)) as ProcessWait);
        Ok(Self {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            wait,
        })
    }
}

async fn supervise(supervisor: Supervisor, process: SupervisedProcess) {
    let Supervisor {
        hub,
        generation,
        accepting,
        events,
    } = supervisor;
    let SupervisedProcess {
        stdout,
        stderr,
        wait,
        stop_receiver,
    } = process;
    let (pump, mut messages) = mpsc::channel(PUMP_CAPACITY);
    let stdout_reader = tokio::spawn(read_output("stdout", stdout, pump.clone()));
    let stderr_reader = tokio::spawn(read_output("stderr", stderr, pump.clone()));
    let process_waiter = tokio::spawn(async move {
        let end = wait(stop_receiver).await;
        let _ = pump.send(PumpMessage::ProcessEnded(end)).await;
    });

    let mut readers_remaining = 2;
    let mut process_end = None;
    let mut pump_failure = None;
    let mut reader_deadline = None;
    while readers_remaining > 0 || process_end.is_none() {
        let message = match reader_deadline {
            Some(deadline) => match timeout_at(deadline, messages.recv()).await {
                Ok(message) => message,
                Err(_) => break,
            },
            None => messages.recv().await,
        };
        let Some(message) = message else {
            pump_failure.get_or_insert_with(|| "terminal supervision channel closed".to_owned());
            break;
        };
        match message {
            PumpMessage::Output(bytes) => {
                if let Some(hub) = hub.upgrade() {
                    hub.transcript
                        .record("output", String::from_utf8_lossy(&bytes).into_owned())
                        .await;
                }
                let _ = events.send(TerminalEvent::Output(bytes));
            }
            PumpMessage::ReaderFailed(message) => {
                pump_failure.get_or_insert(message.clone());
                if let Some(hub) = hub.upgrade() {
                    hub.request_stop(generation, message);
                }
            }
            PumpMessage::ReaderDone => readers_remaining -= 1,
            PumpMessage::ProcessEnded(end) => {
                process_end = Some(end);
                reader_deadline = Some(Instant::now() + READER_DRAIN_GRACE);
            }
        }
    }

    if readers_remaining > 0 {
        stdout_reader.abort();
        stderr_reader.abort();
    }
    let _ = stdout_reader.await;
    let _ = stderr_reader.await;
    if process_end.is_none() {
        process_waiter.abort();
    }
    let _ = process_waiter.await;

    let final_event = match (pump_failure, process_end) {
        (Some(message), _) => TerminalEvent::Failed(message),
        (None, Some(ProcessEnd::Exited(code))) => TerminalEvent::Exited { code },
        (None, Some(ProcessEnd::Failed(message))) => TerminalEvent::Failed(message),
        (None, None) => TerminalEvent::Failed("terminal supervision ended unexpectedly".to_owned()),
    };
    accepting.store(false, Ordering::Release);
    let _ = events.send(final_event);

    if let Some(hub) = hub.upgrade() {
        let mut state = hub.state.lock().expect("terminal hub lock poisoned");
        if state.active.as_ref().map(|active| active.generation) == Some(generation) {
            state.active = None;
        }
        state.processes.remove(&generation);
        drop(state);
        hub.process_finished.notify_waiters();
    }
}

async fn read_output(stream: &str, mut reader: BoxReader, sender: mpsc::Sender<PumpMessage>) {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if sender
                    .send(PumpMessage::Output(buffer[..count].to_vec()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                if sender
                    .send(PumpMessage::ReaderFailed(format!(
                        "could not read terminal {stream}: {error}"
                    )))
                    .await
                    .is_err()
                {
                    return;
                }
                break;
            }
        }
    }
    let _ = sender.send(PumpMessage::ReaderDone).await;
}

async fn wait_for_child(mut child: Child, mut stop: mpsc::Receiver<String>) -> ProcessEnd {
    let end = tokio::select! {
        result = child.wait() => process_status(result),
        failure = stop.recv() => {
            match failure {
                Some(mut message) => {
                    if let Err(error) = child.start_kill() {
                        message.push_str(&format!("; could not stop terminal process: {error}"));
                    }
                    if let Err(error) = child.wait().await {
                        message.push_str(&format!("; could not reap terminal process: {error}"));
                    }
                    ProcessEnd::Failed(message)
                }
                None => {
                    let mut message = "terminal ownership ended; stopping proxy".to_owned();
                    if let Err(error) = child.start_kill() {
                        message.push_str(&format!("; could not stop terminal process: {error}"));
                    }
                    if let Err(error) = child.wait().await {
                        message.push_str(&format!("; could not reap terminal process: {error}"));
                    }
                    ProcessEnd::Failed(message)
                }
            }
        }
    };
    end
}

fn process_status(result: io::Result<std::process::ExitStatus>) -> ProcessEnd {
    match result {
        Ok(status) => ProcessEnd::Exited(status.code()),
        Err(error) => ProcessEnd::Failed(format!("could not wait for terminal process: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, sync::atomic::AtomicUsize};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        time::{Duration, timeout},
    };

    use super::*;

    fn environment() -> Arc<Environment> {
        Environment::for_test()
    }

    fn running_process() -> (TerminalProcess, DuplexStream, DuplexStream) {
        let (process_stdin, test_stdin) = tokio::io::duplex(1024);
        let (test_stdout, process_stdout) = tokio::io::duplex(1024);
        let wait = Box::new(|_stop| Box::pin(std::future::pending()) as ProcessWait);
        (
            TerminalProcess {
                stdin: Box::new(process_stdin),
                stdout: Box::new(process_stdout),
                stderr: Box::new(tokio::io::empty()),
                wait,
            },
            test_stdin,
            test_stdout,
        )
    }

    fn ended_process(end: ProcessEnd) -> TerminalProcess {
        TerminalProcess {
            stdin: Box::new(tokio::io::sink()),
            stdout: Box::new(tokio::io::empty()),
            stderr: Box::new(tokio::io::empty()),
            wait: Box::new(move |_stop| Box::pin(async move { end }) as ProcessWait),
        }
    }

    #[test]
    fn dropping_reservation_releases_admission() {
        let environment = environment();
        drop(reserve(environment.clone()).unwrap());
        assert!(reserve(environment).is_ok());
    }

    #[tokio::test]
    async fn open_failure_releases_admission_once() {
        let environment = environment();
        let mut reservation = reserve(environment.clone()).unwrap();
        let result = reservation
            .open_with(|_| Err(io::Error::new(io::ErrorKind::NotFound, "missing podman")));
        assert!(result.is_err());
        drop(reservation);
        assert!(reserve(environment).is_ok());
    }

    #[tokio::test]
    async fn explicit_close_releases_admission_without_stopping_process() {
        let environment = environment();
        let (process, _, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let attachment = reservation.open_with(|_| Ok(process)).unwrap();
        let error = reserve(environment.clone())
            .err()
            .expect("a second attachment must conflict");
        assert_eq!(error.kind(), RuntimeErrorKind::Conflict);

        attachment.close().await;
        let mut reservation = reserve(environment).unwrap();
        let attachment = reservation
            .open_with(|_| -> io::Result<TerminalProcess> {
                panic!("closing an attachment must not stop its terminal")
            })
            .unwrap();
        drop(attachment);
    }

    #[tokio::test]
    async fn attachment_reconnects_without_replay_and_preserves_raw_output() {
        let environment = environment();
        let (process, mut input, mut output) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = reservation.open_with(|_| Ok(process)).unwrap();

        output.write_all(&[0xff, b'a']).await.unwrap();
        assert_eq!(
            attachment.next_event().await,
            Some(TerminalEvent::Output(vec![0xff, b'a']))
        );
        attachment
            .send(TerminalInput::Text("cd /tmp\n".into()))
            .await
            .unwrap();
        let mut sent = [0; 8];
        input.read_exact(&mut sent).await.unwrap();
        assert_eq!(&sent, b"cd /tmp\n");
        drop(attachment);

        output.write_all(b"while detached\n").await.unwrap();
        wait_for_transcript(&environment, 3).await;

        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = reservation
            .open_with(|_| -> io::Result<TerminalProcess> {
                panic!("a reconnect must not spawn a second terminal")
            })
            .unwrap();
        assert!(
            timeout(Duration::from_millis(20), attachment.next_event())
                .await
                .is_err(),
            "detached output must not be replayed"
        );

        attachment
            .send(TerminalInput::Binary(b"echo $VALUE\n".to_vec()))
            .await
            .unwrap();
        let mut sent = [0; 12];
        input.read_exact(&mut sent).await.unwrap();
        assert_eq!(&sent, b"echo $VALUE\n");
        output.write_all(b"new output\n").await.unwrap();
        assert_eq!(
            attachment.next_event().await,
            Some(TerminalEvent::Output(b"new output\n".to_vec()))
        );

        let transcript = environment.transcript_snapshot().await;
        assert_eq!(transcript[0].data, "�a");
        assert_eq!(transcript[1].data, "cd /tmp\n");
        assert_eq!(transcript[2].data, "while detached\n");
    }

    #[tokio::test]
    async fn exit_and_failure_allow_a_fresh_terminal() {
        for expected in [
            TerminalEvent::Exited { code: Some(7) },
            TerminalEvent::Failed("proxy wait failed".into()),
        ] {
            let environment = environment();
            let end = match &expected {
                TerminalEvent::Exited { code } => ProcessEnd::Exited(*code),
                TerminalEvent::Failed(message) => ProcessEnd::Failed(message.clone()),
                TerminalEvent::Output(_) => unreachable!(),
            };
            let mut reservation = reserve(environment.clone()).unwrap();
            let mut attachment = reservation.open_with(|_| Ok(ended_process(end))).unwrap();
            assert_eq!(attachment.next_event().await, Some(expected));
            drop(attachment);
            environment.terminal().wait_for_processes().await;

            let spawns = Arc::new(AtomicUsize::new(0));
            let counted = spawns.clone();
            let (process, _, _) = running_process();
            let mut reservation = reserve(environment).unwrap();
            let attachment = reservation
                .open_with(|_| {
                    counted.fetch_add(1, Ordering::Relaxed);
                    Ok(process)
                })
                .unwrap();
            assert_eq!(spawns.load(Ordering::Relaxed), 1);
            drop(attachment);
        }
    }

    #[tokio::test]
    async fn receiver_lag_is_an_explicit_failure() {
        let environment = environment();
        let (process, _, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = reservation.open_with(|_| Ok(process)).unwrap();
        {
            let state = environment
                .terminal()
                .inner
                .state
                .lock()
                .expect("terminal hub lock poisoned");
            let events = &state.active.as_ref().unwrap().events;
            for index in 0..=EVENT_CAPACITY {
                let _ = events.send(TerminalEvent::Output(vec![index as u8]));
            }
        }
        assert!(matches!(
            attachment.next_event().await,
            Some(TerminalEvent::Failed(message)) if message.contains("missed 1 events")
        ));
    }

    #[tokio::test]
    async fn process_exit_aborts_a_reader_that_never_closes() {
        let environment = environment();
        let (held_stdout, process_stdout) = tokio::io::duplex(64);
        let process = TerminalProcess {
            stdin: Box::new(tokio::io::sink()),
            stdout: Box::new(process_stdout),
            stderr: Box::new(tokio::io::empty()),
            wait: Box::new(|_stop| Box::pin(async { ProcessEnd::Exited(Some(0)) }) as ProcessWait),
        };
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = reservation.open_with(|_| Ok(process)).unwrap();

        assert_eq!(
            timeout(Duration::from_secs(1), attachment.next_event())
                .await
                .expect("reader drain must be bounded"),
            Some(TerminalEvent::Exited { code: Some(0) })
        );
        timeout(
            Duration::from_millis(50),
            environment.terminal().wait_for_processes(),
        )
        .await
        .expect("finished proxy must leave the hub");
        drop(held_stdout);
    }

    #[tokio::test]
    async fn container_removal_forces_and_reaps_a_lingering_proxy() {
        let environment = environment();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_by_waiter = stopped.clone();
        let process = TerminalProcess {
            stdin: Box::new(tokio::io::sink()),
            stdout: Box::new(tokio::io::empty()),
            stderr: Box::new(tokio::io::empty()),
            wait: Box::new(move |mut stop| {
                Box::pin(async move {
                    let message = stop.recv().await.expect("hub must request proxy stop");
                    stopped_by_waiter.store(true, Ordering::Release);
                    ProcessEnd::Failed(message)
                }) as ProcessWait
            }),
        };
        let mut reservation = reserve(environment.clone()).unwrap();
        let attachment = reservation.open_with(|_| Ok(process)).unwrap();
        drop(attachment);

        assert!(
            environment
                .terminal()
                .finish_with_timeouts(Duration::from_millis(5), Duration::from_millis(100))
                .await
        );
        assert!(stopped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn process_cleanup_deadlines_remain_bounded() {
        let environment = environment();
        let (process, _, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let attachment = reservation.open_with(|_| Ok(process)).unwrap();
        drop(attachment);

        let settled = timeout(Duration::from_millis(100), async {
            environment
                .terminal()
                .finish_with_timeouts(Duration::from_millis(5), Duration::from_millis(5))
                .await
        })
        .await
        .expect("cleanup must honor both deadlines");
        assert!(!settled, "an unresponsive fake proxy cannot report reaped");
    }

    #[tokio::test]
    async fn failed_input_is_not_recorded_as_terminal_evidence() {
        let environment = environment();
        let (process_stdin, closed_peer) = tokio::io::duplex(64);
        drop(closed_peer);
        let process = TerminalProcess {
            stdin: Box::new(process_stdin),
            stdout: Box::new(tokio::io::empty()),
            stderr: Box::new(tokio::io::empty()),
            wait: Box::new(|mut stop| {
                Box::pin(async move {
                    ProcessEnd::Failed(stop.recv().await.unwrap_or_else(|| "stopped".into()))
                }) as ProcessWait
            }),
        };
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = reservation.open_with(|_| Ok(process)).unwrap();

        assert!(
            attachment
                .send(TerminalInput::Text("never executed\n".into()))
                .await
                .is_err()
        );
        assert!(environment.transcript_snapshot().await.is_empty());
        drop(attachment);
        environment.terminal().stop_and_reap().await;
    }

    async fn wait_for_transcript(environment: &Environment, length: usize) {
        timeout(Duration::from_secs(1), async {
            loop {
                if environment.transcript_snapshot().await.len() >= length {
                    return Ok::<_, Infallible>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("transcript was not recorded")
        .unwrap();
    }
}
