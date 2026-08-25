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
    activity::{ActivityLog, ExecutionActivity, TerminalInputKind},
    environment::{Environment, Transcript},
    error::{RuntimeError, RuntimeErrorKind},
    podman,
};

const EVENT_CAPACITY: usize = 32;
const PUMP_CAPACITY: usize = 32;
const PROCESS_DRAIN_GRACE: Duration = Duration::from_millis(500);
const PROCESS_FORCE_GRACE: Duration = Duration::from_secs(1);
const READER_DRAIN_GRACE: Duration = Duration::from_millis(250);
const MIN_TERMINAL_DIMENSION: u16 = 1;
const MAX_TERMINAL_DIMENSION: u16 = 1000;

type BoxReader = Box<dyn AsyncRead + Unpin + Send>;
type BoxWriter = Box<dyn AsyncWrite + Unpin + Send>;
type ProcessWait = Pin<Box<dyn Future<Output = ProcessEnd> + Send>>;
type WaitProcess = Box<dyn FnOnce(mpsc::Receiver<String>) -> ProcessWait + Send>;

pub enum TerminalInput {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalDimensions {
    columns: u16,
    rows: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTerminalDimensions {
    columns: u16,
    rows: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Output(Vec<u8>),
    OutputGap { missed: u64 },
    Exited { code: Option<i32> },
    Failed(String),
}

pub struct TerminalReservation {
    environment: Option<Arc<Environment>>,
}

pub struct TerminalAttachment {
    environment: Option<Arc<Environment>>,
    generation: u64,
    resumed: bool,
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
    activity: Arc<ActivityLog>,
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
    pty_path: String,
}

struct AttachmentParts {
    generation: u64,
    resumed: bool,
    stdin: Arc<Mutex<BoxWriter>>,
    events: broadcast::Receiver<TerminalEvent>,
}

struct TerminalProcess {
    stdin: BoxWriter,
    stdout: BoxReader,
    stderr: BoxReader,
    wait: WaitProcess,
}

struct PreparedTerminal {
    process: TerminalProcess,
    pty_path: String,
}

struct SupervisedProcess {
    stdout: BoxReader,
    stderr: BoxReader,
    wait: WaitProcess,
    stop_receiver: mpsc::Receiver<String>,
}

struct Supervisor {
    hub: Weak<HubInner>,
    activity: Arc<ActivityLog>,
    generation: u64,
    accepting: Arc<AtomicBool>,
    events: broadcast::Sender<TerminalEvent>,
}

enum PumpMessage {
    Output(OutputStream, Vec<u8>),
    ReaderFailed(String),
    ReaderDone(OutputStream),
    ProcessEnded(ProcessEnd),
}

#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Default)]
struct Utf8TranscriptDecoder {
    pending: Vec<u8>,
}

enum ProcessEnd {
    Exited(Option<i32>),
    Failed(String),
}

impl TerminalDimensions {
    pub fn new(columns: u16, rows: u16) -> Result<Self, InvalidTerminalDimensions> {
        if (MIN_TERMINAL_DIMENSION..=MAX_TERMINAL_DIMENSION).contains(&columns)
            && (MIN_TERMINAL_DIMENSION..=MAX_TERMINAL_DIMENSION).contains(&rows)
        {
            Ok(Self { columns, rows })
        } else {
            Err(InvalidTerminalDimensions { columns, rows })
        }
    }

    pub fn columns(self) -> u16 {
        self.columns
    }

    pub fn rows(self) -> u16 {
        self.rows
    }
}

impl fmt::Display for InvalidTerminalDimensions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "terminal dimensions must each be in {MIN_TERMINAL_DIMENSION}..={MAX_TERMINAL_DIMENSION} (columns {}, rows {})",
            self.columns, self.rows
        )
    }
}

impl std::error::Error for InvalidTerminalDimensions {}

impl Utf8TranscriptDecoder {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0;
        let mut decoded = String::new();

        loop {
            let remaining = &self.pending[consumed..];
            if remaining.is_empty() {
                break;
            }
            match std::str::from_utf8(remaining) {
                Ok(valid) => {
                    decoded.push_str(valid);
                    consumed = self.pending.len();
                    break;
                }
                Err(error) => {
                    let valid_end = error.valid_up_to();
                    if valid_end > 0 {
                        decoded.push_str(
                            std::str::from_utf8(&remaining[..valid_end])
                                .expect("UTF-8 validator identified a valid prefix"),
                        );
                        consumed += valid_end;
                    }
                    let Some(invalid_length) = error.error_len() else {
                        break;
                    };
                    decoded.push(char::REPLACEMENT_CHARACTER);
                    consumed += invalid_length;
                }
            }
        }

        self.pending.drain(..consumed);
        decoded
    }

    fn finish(&mut self) -> String {
        let mut decoded = self.push(&[]);
        if !self.pending.is_empty() {
            decoded.push(char::REPLACEMENT_CHARACTER);
            self.pending.clear();
        }
        decoded
    }
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
    pub async fn open(
        mut self,
        initial: TerminalDimensions,
    ) -> Result<TerminalAttachment, TerminalOpenError> {
        self.open_with(
            initial,
            |container, generation, dimensions| async move {
                let opened = podman::open_terminal(&container, generation, dimensions).await?;
                Ok(PreparedTerminal {
                    process: TerminalProcess::from_child(opened.child)?,
                    pty_path: opened.pty_path,
                })
            },
            |container, pty_path, dimensions| async move {
                podman::resize_terminal(&container, &pty_path, dimensions).await
            },
        )
        .await
    }

    async fn open_with<Open, OpenFuture, Resize, ResizeFuture>(
        &mut self,
        initial: TerminalDimensions,
        open_fresh: Open,
        resize: Resize,
    ) -> Result<TerminalAttachment, TerminalOpenError>
    where
        Open: FnOnce(String, u64, TerminalDimensions) -> OpenFuture,
        OpenFuture: Future<Output = io::Result<PreparedTerminal>>,
        Resize: Fn(String, String, TerminalDimensions) -> ResizeFuture,
        ResizeFuture: Future<Output = io::Result<()>>,
    {
        let environment = self
            .environment
            .as_ref()
            .cloned()
            .expect("terminal reservation owns its environment");
        let parts = match environment
            .terminal()
            .attach_with(environment.container_name(), initial, open_fresh, resize)
            .await
        {
            Ok(parts) => parts,
            Err(error) => return Err(TerminalOpenError(error)),
        };
        self.environment.take();

        Ok(TerminalAttachment {
            environment: Some(environment),
            generation: parts.generation,
            resumed: parts.resumed,
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
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    pub async fn send(&mut self, input: TerminalInput) -> io::Result<()> {
        let environment = self
            .environment
            .as_ref()
            .expect("open terminal attachment owns its environment");
        let (transcript, bytes, mut input_kind) = match input {
            TerminalInput::Text(text) => (text.clone(), text.into_bytes(), TerminalInputKind::Text),
            TerminalInput::Binary(bytes) => (
                String::from_utf8_lossy(&bytes).into_owned(),
                bytes,
                TerminalInputKind::Binary,
            ),
        };
        if bytes == [0x03] {
            input_kind = TerminalInputKind::Interrupt;
        }

        let result = self.stdin.lock().await.write_all(&bytes).await;
        match &result {
            Ok(()) => {
                environment.record_activity(ExecutionActivity::TerminalInput {
                    generation: self.generation,
                    input_kind,
                    bytes: bytes.len() as u64,
                });
                environment.record_transcript("input", transcript).await;
            }
            Err(error) => environment.terminal().request_failure(
                self.generation,
                format!("could not write terminal input: {error}"),
            ),
        }
        result
    }

    pub async fn resize(&mut self, dimensions: TerminalDimensions) -> io::Result<()> {
        let environment = self
            .environment
            .as_ref()
            .expect("open terminal attachment owns its environment");
        environment
            .terminal()
            .resize(environment.container_name(), self.generation, dimensions)
            .await
    }

    pub async fn next_event(&mut self) -> Option<TerminalEvent> {
        match self.events.recv().await {
            Ok(event) => Some(event),
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                Some(TerminalEvent::OutputGap { missed })
            }
            Err(broadcast::error::RecvError::Closed) => None,
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
    pub(crate) fn new(transcript: Arc<Transcript>, activity: Arc<ActivityLog>) -> Self {
        Self {
            inner: Arc::new(HubInner {
                transcript,
                activity,
                state: std::sync::Mutex::new(HubState {
                    next_generation: 1,
                    active: None,
                    processes: HashMap::new(),
                }),
                process_finished: Notify::new(),
            }),
        }
    }

    async fn attach_with<Open, OpenFuture, Resize, ResizeFuture>(
        &self,
        container: &str,
        dimensions: TerminalDimensions,
        open_fresh: Open,
        resize: Resize,
    ) -> io::Result<AttachmentParts>
    where
        Open: FnOnce(String, u64, TerminalDimensions) -> OpenFuture,
        OpenFuture: Future<Output = io::Result<PreparedTerminal>>,
        Resize: Fn(String, String, TerminalDimensions) -> ResizeFuture,
        ResizeFuture: Future<Output = io::Result<()>>,
    {
        if let Some(parts) = self.active_attachment(true) {
            match self
                .resize_with(container, parts.generation, dimensions, &resize)
                .await
            {
                Ok(()) => return Ok(parts),
                Err(error) if self.generation_is_live(parts.generation) => return Err(error),
                Err(_) => {}
            }
        }

        let handle = tokio::runtime::Handle::try_current().map_err(|error| {
            io::Error::other(format!(
                "terminal must open inside a Tokio runtime: {error}"
            ))
        })?;
        let generation = {
            let mut state = self.inner.state.lock().expect("terminal hub lock poisoned");
            let generation = state.next_generation;
            state.next_generation += 1;
            generation
        };
        let prepared = open_fresh(container.to_owned(), generation, dimensions).await?;
        let PreparedTerminal { process, pty_path } = prepared;
        let TerminalProcess {
            stdin: process_stdin,
            stdout,
            stderr,
            wait,
        } = process;
        let accepting = Arc::new(AtomicBool::new(true));
        let stdin = Arc::new(Mutex::new(process_stdin));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let event_receiver = events.subscribe();
        let (stop, stop_receiver) = mpsc::channel(1);
        let mut state = self.inner.state.lock().expect("terminal hub lock poisoned");
        state.processes.insert(generation, stop);

        state.active = Some(ActiveTerminal {
            generation,
            accepting: accepting.clone(),
            stdin: stdin.clone(),
            events: events.clone(),
            pty_path,
        });
        drop(state);
        self.inner.activity.record(ExecutionActivity::ShellStarted {
            generation,
            columns: dimensions.columns(),
            rows: dimensions.rows(),
        });
        handle.spawn(supervise(
            Supervisor {
                hub: Arc::downgrade(&self.inner),
                activity: self.inner.activity.clone(),
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
            resumed: false,
            stdin,
            events: event_receiver,
        })
    }

    fn active_attachment(&self, resumed: bool) -> Option<AttachmentParts> {
        let state = self.inner.state.lock().expect("terminal hub lock poisoned");
        let active = state.active.as_ref()?;
        if !active.accepting.load(Ordering::Acquire) {
            return None;
        }
        Some(AttachmentParts {
            generation: active.generation,
            resumed,
            stdin: active.stdin.clone(),
            events: active.events.subscribe(),
        })
    }

    async fn resize(
        &self,
        container: &str,
        generation: u64,
        dimensions: TerminalDimensions,
    ) -> io::Result<()> {
        self.resize_with(
            container,
            generation,
            dimensions,
            &|container, pty_path, dimensions| async move {
                podman::resize_terminal(&container, &pty_path, dimensions).await
            },
        )
        .await
    }

    async fn resize_with<Resize, ResizeFuture>(
        &self,
        container: &str,
        generation: u64,
        dimensions: TerminalDimensions,
        resize: &Resize,
    ) -> io::Result<()>
    where
        Resize: Fn(String, String, TerminalDimensions) -> ResizeFuture,
        ResizeFuture: Future<Output = io::Result<()>>,
    {
        let pty_path = {
            let state = self.inner.state.lock().expect("terminal hub lock poisoned");
            let active = state.active.as_ref().filter(|active| {
                active.generation == generation && active.accepting.load(Ordering::Acquire)
            });
            active
                .map(|active| active.pty_path.clone())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotConnected,
                        "terminal generation is not live",
                    )
                })?
        };

        resize(container.to_owned(), pty_path, dimensions).await?;
        let state = self.inner.state.lock().expect("terminal hub lock poisoned");
        let generation_is_live = state.active.as_ref().is_some_and(|active| {
            active.generation == generation && active.accepting.load(Ordering::Acquire)
        });
        if !generation_is_live {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "terminal generation ended during resize",
            ));
        }
        self.inner
            .activity
            .record(ExecutionActivity::TerminalResized {
                generation,
                columns: dimensions.columns(),
                rows: dimensions.rows(),
            });
        drop(state);
        Ok(())
    }

    fn generation_is_live(&self, generation: u64) -> bool {
        let state = self.inner.state.lock().expect("terminal hub lock poisoned");
        state.active.as_ref().is_some_and(|active| {
            active.generation == generation && active.accepting.load(Ordering::Acquire)
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
        activity,
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
    let stdout_reader = tokio::spawn(read_output(OutputStream::Stdout, stdout, pump.clone()));
    let stderr_reader = tokio::spawn(read_output(OutputStream::Stderr, stderr, pump.clone()));
    let process_waiter = tokio::spawn(async move {
        let end = wait(stop_receiver).await;
        let _ = pump.send(PumpMessage::ProcessEnded(end)).await;
    });

    let mut readers_remaining = 2;
    let mut process_end = None;
    let mut pump_failure = None;
    let mut reader_deadline = None;
    let mut stdout_decoder = Utf8TranscriptDecoder::default();
    let mut stderr_decoder = Utf8TranscriptDecoder::default();
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
            PumpMessage::Output(stream, bytes) => {
                let decoder = match stream {
                    OutputStream::Stdout => &mut stdout_decoder,
                    OutputStream::Stderr => &mut stderr_decoder,
                };
                let transcript = decoder.push(&bytes);
                if let Some(hub) = hub.upgrade() {
                    if !transcript.is_empty() {
                        hub.transcript.record("output", transcript).await;
                    }
                }
                let _ = events.send(TerminalEvent::Output(bytes));
            }
            PumpMessage::ReaderFailed(message) => {
                pump_failure.get_or_insert(message.clone());
                if let Some(hub) = hub.upgrade() {
                    hub.request_stop(generation, message);
                }
            }
            PumpMessage::ReaderDone(stream) => {
                let transcript = match stream {
                    OutputStream::Stdout => stdout_decoder.finish(),
                    OutputStream::Stderr => stderr_decoder.finish(),
                };
                if let Some(hub) = hub.upgrade()
                    && !transcript.is_empty()
                {
                    hub.transcript.record("output", transcript).await;
                }
                readers_remaining -= 1;
            }
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

    for transcript in [stdout_decoder.finish(), stderr_decoder.finish()] {
        if let Some(hub) = hub.upgrade()
            && !transcript.is_empty()
        {
            hub.transcript.record("output", transcript).await;
        }
    }

    let final_event = match (pump_failure, process_end) {
        (Some(message), _) => TerminalEvent::Failed(message),
        (None, Some(ProcessEnd::Exited(code))) => TerminalEvent::Exited { code },
        (None, Some(ProcessEnd::Failed(message))) => TerminalEvent::Failed(message),
        (None, None) => TerminalEvent::Failed("terminal supervision ended unexpectedly".to_owned()),
    };
    accepting.store(false, Ordering::Release);
    let hub = hub.upgrade();
    if let Some(hub) = &hub {
        let mut state = hub.state.lock().expect("terminal hub lock poisoned");
        record_shell_end(&activity, generation, &final_event);
        if state.active.as_ref().map(|active| active.generation) == Some(generation) {
            state.active = None;
        }
        state.processes.remove(&generation);
        drop(state);
    } else {
        record_shell_end(&activity, generation, &final_event);
    }
    let _ = events.send(final_event);

    if let Some(hub) = hub {
        hub.process_finished.notify_waiters();
    }
}

fn record_shell_end(activity: &ActivityLog, generation: u64, final_event: &TerminalEvent) {
    match final_event {
        TerminalEvent::Exited { code } => activity.record(ExecutionActivity::ShellExited {
            generation,
            code: *code,
        }),
        TerminalEvent::Failed(_) => activity.record(ExecutionActivity::ShellFailed { generation }),
        TerminalEvent::Output(_) | TerminalEvent::OutputGap { .. } => {
            unreachable!("supervisor final event must end the shell")
        }
    }
}

async fn read_output(
    stream: OutputStream,
    mut reader: BoxReader,
    sender: mpsc::Sender<PumpMessage>,
) {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if sender
                    .send(PumpMessage::Output(stream, buffer[..count].to_vec()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                if sender
                    .send(PumpMessage::ReaderFailed(format!(
                        "could not read terminal {}: {error}",
                        match stream {
                            OutputStream::Stdout => "stdout",
                            OutputStream::Stderr => "stderr",
                        }
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
    let _ = sender.send(PumpMessage::ReaderDone(stream)).await;
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
    use std::{
        convert::Infallible,
        sync::{Mutex as StdMutex, atomic::AtomicUsize},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        sync::oneshot,
        time::{Duration, timeout},
    };

    use super::*;

    fn environment() -> Arc<Environment> {
        Environment::for_test()
    }

    fn dimensions() -> TerminalDimensions {
        TerminalDimensions::new(80, 24).unwrap()
    }

    fn prepared(process: TerminalProcess) -> PreparedTerminal {
        PreparedTerminal {
            process,
            pty_path: "/dev/pts/7".into(),
        }
    }

    async fn open_process(
        reservation: &mut TerminalReservation,
        process: TerminalProcess,
    ) -> Result<TerminalAttachment, TerminalOpenError> {
        reservation
            .open_with(
                dimensions(),
                move |_, _, _| async move { Ok(prepared(process)) },
                |_, _, _| async { Ok(()) },
            )
            .await
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

    fn activities(environment: &Environment) -> Vec<ExecutionActivity> {
        environment
            .activity_snapshot()
            .0
            .into_iter()
            .map(|event| event.activity)
            .collect()
    }

    #[test]
    fn terminal_dimensions_enforce_inclusive_bounds() {
        for (columns, rows) in [(1, 1), (1, 1000), (1000, 1), (1000, 1000)] {
            let dimensions = TerminalDimensions::new(columns, rows).unwrap();
            assert_eq!(dimensions.columns(), columns);
            assert_eq!(dimensions.rows(), rows);
        }
        for (columns, rows) in [(0, 1), (1, 0), (1001, 1), (1, 1001)] {
            assert_eq!(
                TerminalDimensions::new(columns, rows),
                Err(InvalidTerminalDimensions { columns, rows })
            );
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
            .open_with(
                dimensions(),
                |_, _, _| async { Err(io::Error::new(io::ErrorKind::NotFound, "missing podman")) },
                |_, _, _| async { Ok(()) },
            )
            .await;
        assert!(result.is_err());
        assert!(activities(&environment).is_empty());
        drop(reservation);
        assert!(reserve(environment).is_ok());
    }

    #[tokio::test]
    async fn explicit_close_releases_admission_without_stopping_process() {
        let environment = environment();
        let (process, _, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let attachment = open_process(&mut reservation, process).await.unwrap();
        let error = reserve(environment.clone())
            .err()
            .expect("a second attachment must conflict");
        assert_eq!(error.kind(), RuntimeErrorKind::Conflict);

        attachment.close().await;
        let mut reservation = reserve(environment.clone()).unwrap();
        let (unused_process, _, _) = running_process();
        let attachment = open_process(&mut reservation, unused_process)
            .await
            .unwrap();
        drop(attachment);
    }

    #[tokio::test]
    async fn fresh_and_resumed_open_apply_requested_size_to_one_generation() {
        let environment = environment();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let spawns = Arc::new(AtomicUsize::new(0));
        let (process, _, _) = running_process();

        let mut reservation = reserve(environment.clone()).unwrap();
        let fresh_calls = calls.clone();
        let fresh_spawns = spawns.clone();
        let first_size = TerminalDimensions::new(132, 43).unwrap();
        let first = reservation
            .open_with(
                first_size,
                move |_, _, dimensions| async move {
                    fresh_spawns.fetch_add(1, Ordering::Relaxed);
                    fresh_calls.lock().unwrap().push((
                        "fresh",
                        dimensions,
                        "/dev/pts/41".to_owned(),
                    ));
                    Ok(PreparedTerminal {
                        process,
                        pty_path: "/dev/pts/41".into(),
                    })
                },
                |_, _, _| async { Ok(()) },
            )
            .await
            .unwrap();
        assert!(!first.resumed());
        assert_eq!(spawns.load(Ordering::Relaxed), 1);
        assert_eq!(calls.lock().unwrap().len(), 1);
        let generation = first.generation;
        drop(first);

        let (unused_process, _, _) = running_process();
        let resumed_calls = calls.clone();
        let resumed_spawns = spawns.clone();
        let second_size = TerminalDimensions::new(151, 47).unwrap();
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut second = reservation
            .open_with(
                second_size,
                move |_, _, _| async move {
                    resumed_spawns.fetch_add(1, Ordering::Relaxed);
                    Ok(prepared(unused_process))
                },
                move |_, path, dimensions| {
                    let calls = resumed_calls.clone();
                    async move {
                        calls.lock().unwrap().push(("resize", dimensions, path));
                        Ok(())
                    }
                },
            )
            .await
            .unwrap();
        assert!(second.resumed());
        assert_eq!(spawns.load(Ordering::Relaxed), 1);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                ("fresh", first_size, "/dev/pts/41".to_owned()),
                ("resize", second_size, "/dev/pts/41".to_owned()),
            ]
        );
        assert!(
            timeout(Duration::from_millis(20), second.next_event())
                .await
                .is_err(),
            "reattach must not replay output"
        );
        assert_eq!(
            activities(&environment),
            [
                ExecutionActivity::ShellStarted {
                    generation,
                    columns: first_size.columns(),
                    rows: first_size.rows(),
                },
                ExecutionActivity::TerminalResized {
                    generation,
                    columns: second_size.columns(),
                    rows: second_size.rows(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn attachment_reconnects_without_replay_and_preserves_raw_output() {
        let environment = environment();
        let (process, mut input, mut output) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = open_process(&mut reservation, process).await.unwrap();
        assert!(!attachment.resumed());

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
        let (unused_process, _, _) = running_process();
        let mut attachment = open_process(&mut reservation, unused_process)
            .await
            .unwrap();
        assert!(attachment.resumed());
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
    async fn stale_resize_cannot_target_a_new_generation_with_the_same_path() {
        let environment = environment();
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut first = open_process(&mut reservation, ended_process(ProcessEnd::Exited(Some(0))))
            .await
            .unwrap();
        let stale_generation = first.generation;
        assert_eq!(
            first.next_event().await,
            Some(TerminalEvent::Exited { code: Some(0) })
        );
        drop(first);
        environment.terminal().wait_for_processes().await;

        let (process, _, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let second = open_process(&mut reservation, process).await.unwrap();
        assert_ne!(second.generation, stale_generation);

        let calls = Arc::new(AtomicUsize::new(0));
        let resize_calls = calls.clone();
        let result = environment
            .terminal()
            .resize_with("none", stale_generation, dimensions(), &move |_, _, _| {
                resize_calls.fetch_add(1, Ordering::Relaxed);
                std::future::ready(Ok(()))
            })
            .await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotConnected);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        drop(second);
    }

    #[tokio::test]
    async fn resize_failure_is_explicit_and_never_recorded() {
        let environment = environment();
        let (process, _, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let attachment = open_process(&mut reservation, process).await.unwrap();

        let error = environment
            .terminal()
            .resize_with("none", attachment.generation, dimensions(), &|_, _, _| {
                std::future::ready(Err(io::Error::other("stty rejected resize")))
            })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "stty rejected resize");
        assert!(environment.transcript_snapshot().await.is_empty());
        assert_eq!(
            activities(&environment),
            [ExecutionActivity::ShellStarted {
                generation: attachment.generation,
                columns: dimensions().columns(),
                rows: dimensions().rows(),
            }]
        );
        drop(attachment);
    }

    #[tokio::test]
    async fn raw_etx_is_written_and_recorded_only_after_success() {
        let environment = environment();
        let (process, mut input, _) = running_process();
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = open_process(&mut reservation, process).await.unwrap();

        attachment
            .send(TerminalInput::Binary(vec![0x03]))
            .await
            .unwrap();
        let mut sent = [0_u8; 1];
        input.read_exact(&mut sent).await.unwrap();
        assert_eq!(sent, [0x03]);
        wait_for_transcript(&environment, 1).await;
        let transcript = environment.transcript_snapshot().await;
        assert_eq!(transcript.len(), 1);
        assert_eq!(transcript[0].direction, "input");
        assert_eq!(transcript[0].data.as_bytes(), [0x03]);
        assert_eq!(
            activities(&environment),
            [
                ExecutionActivity::ShellStarted {
                    generation: attachment.generation,
                    columns: dimensions().columns(),
                    rows: dimensions().rows(),
                },
                ExecutionActivity::TerminalInput {
                    generation: attachment.generation,
                    input_kind: TerminalInputKind::Interrupt,
                    bytes: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn output_is_drained_before_the_exit_event() {
        let environment = environment();
        let (test_stdout, process_stdout) = tokio::io::duplex(64);
        let (finish, finished) = oneshot::channel();
        let process = TerminalProcess {
            stdin: Box::new(tokio::io::sink()),
            stdout: Box::new(process_stdout),
            stderr: Box::new(tokio::io::empty()),
            wait: Box::new(move |_stop| {
                Box::pin(async move {
                    finished.await.unwrap();
                    ProcessEnd::Exited(Some(9))
                }) as ProcessWait
            }),
        };
        let mut reservation = reserve(environment.clone()).unwrap();
        let mut attachment = open_process(&mut reservation, process).await.unwrap();

        let mut test_stdout = test_stdout;
        test_stdout.write_all(b"last bytes").await.unwrap();
        test_stdout.shutdown().await.unwrap();
        finish.send(()).unwrap();
        assert_eq!(
            attachment.next_event().await,
            Some(TerminalEvent::Output(b"last bytes".to_vec()))
        );
        assert_eq!(
            attachment.next_event().await,
            Some(TerminalEvent::Exited { code: Some(9) })
        );
        assert_eq!(
            activities(&environment),
            [
                ExecutionActivity::ShellStarted {
                    generation: attachment.generation,
                    columns: dimensions().columns(),
                    rows: dimensions().rows(),
                },
                ExecutionActivity::ShellExited {
                    generation: attachment.generation,
                    code: Some(9),
                },
            ]
        );
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
                TerminalEvent::Output(_) | TerminalEvent::OutputGap { .. } => unreachable!(),
            };
            let mut reservation = reserve(environment.clone()).unwrap();
            let mut attachment = open_process(&mut reservation, ended_process(end))
                .await
                .unwrap();
            assert_eq!(attachment.next_event().await, Some(expected));
            drop(attachment);
            environment.terminal().wait_for_processes().await;

            let spawns = Arc::new(AtomicUsize::new(0));
            let counted = spawns.clone();
            let (process, _, _) = running_process();
            let mut reservation = reserve(environment).unwrap();
            let attachment = reservation
                .open_with(
                    dimensions(),
                    move |_, _, _| async move {
                        counted.fetch_add(1, Ordering::Relaxed);
                        Ok(prepared(process))
                    },
                    |_, _, _| async { Ok(()) },
                )
                .await
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
        let mut attachment = open_process(&mut reservation, process).await.unwrap();
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
            Some(TerminalEvent::OutputGap { missed: 1 })
        ));
    }

    #[test]
    fn transcript_decoder_preserves_split_utf8_and_marks_invalid_bytes() {
        let mut decoder = Utf8TranscriptDecoder::default();
        assert_eq!(decoder.push(&[0xe2, 0x82]), "");
        assert_eq!(decoder.push(&[0xac, b'\n']), "€\n");
        assert_eq!(decoder.push(&[0xff, b'a']), "�a");
        assert_eq!(decoder.push(&[0xf0, 0x9f]), "");
        assert_eq!(decoder.finish(), "�");
        assert_eq!(decoder.finish(), "");
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
        let mut attachment = open_process(&mut reservation, process).await.unwrap();

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
        let attachment = open_process(&mut reservation, process).await.unwrap();
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
        let attachment = open_process(&mut reservation, process).await.unwrap();
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
    async fn failed_input_is_not_recorded_and_emits_authoritative_failure() {
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
        let mut attachment = open_process(&mut reservation, process).await.unwrap();

        let write_error = attachment
            .send(TerminalInput::Text("never executed\n".into()))
            .await
            .expect_err("the closed stdin peer must reject terminal input");
        assert!(environment.transcript_snapshot().await.is_empty());
        assert_eq!(
            timeout(Duration::from_secs(1), attachment.next_event())
                .await
                .expect("a failed input write must produce a terminal ending"),
            Some(TerminalEvent::Failed(format!(
                "could not write terminal input: {write_error}"
            )))
        );
        assert_eq!(
            activities(&environment),
            [
                ExecutionActivity::ShellStarted {
                    generation: attachment.generation,
                    columns: dimensions().columns(),
                    rows: dimensions().rows(),
                },
                ExecutionActivity::ShellFailed {
                    generation: attachment.generation,
                },
            ]
        );
        drop(attachment);
        timeout(
            Duration::from_secs(1),
            environment.terminal().wait_for_processes(),
        )
        .await
        .expect("the failed terminal proxy must be reaped");
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
