use std::{fmt, io, sync::Arc};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin},
    sync::mpsc,
};

use crate::{
    environment::Environment,
    error::{RuntimeError, RuntimeErrorKind},
    podman,
};

pub enum TerminalInput {
    Text(String),
    Binary(Vec<u8>),
}

pub struct TerminalOutput(String);

impl TerminalOutput {
    pub fn into_text(self) -> String {
        self.0
    }
}

pub struct TerminalReservation {
    environment: Option<Arc<Environment>>,
}

pub struct TerminalSession {
    environment: Arc<Environment>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    output_rx: mpsc::Receiver<String>,
}

#[derive(Debug)]
pub struct TerminalOpenError(io::Error);

impl fmt::Display for TerminalOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for TerminalOpenError {}

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
    pub fn open(mut self) -> Result<TerminalSession, TerminalOpenError> {
        let environment = self
            .environment
            .take()
            .expect("terminal reservation owns its environment");
        let mut child = match podman::spawn_terminal(environment.container_name()) {
            Ok(child) => child,
            Err(error) => {
                environment.close_terminal();
                return Err(TerminalOpenError(error));
            }
        };

        let stdin = child.stdin.take().expect("terminal stdin was piped");
        let stdout = child.stdout.take().expect("terminal stdout was piped");
        let stderr = child.stderr.take().expect("terminal stderr was piped");
        let (output_tx, output_rx) = mpsc::channel::<String>(32);
        tokio::spawn(read_output(stdout, output_tx.clone()));
        tokio::spawn(read_output(stderr, output_tx));

        Ok(TerminalSession {
            environment,
            child: Some(child),
            stdin: Some(stdin),
            output_rx,
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

impl TerminalSession {
    pub async fn send(&mut self, input: TerminalInput) -> io::Result<()> {
        let (transcript, bytes) = match input {
            TerminalInput::Text(text) => (text.clone(), text.into_bytes()),
            TerminalInput::Binary(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), bytes),
        };
        self.environment
            .record_transcript("input", transcript)
            .await;
        self.stdin
            .as_mut()
            .expect("open terminal owns stdin")
            .write_all(&bytes)
            .await
    }

    pub async fn next_output(&mut self) -> Option<TerminalOutput> {
        let text = self.output_rx.recv().await?;
        self.environment
            .record_transcript("output", text.clone())
            .await;
        Some(TerminalOutput(text))
    }

    pub async fn close(mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.shutdown().await;
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        self.environment.close_terminal();
    }
}

async fn read_output(mut reader: impl AsyncRead + Unpin, sender: mpsc::Sender<String>) {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if sender
                    .send(String::from_utf8_lossy(&buffer[..count]).into_owned())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}
