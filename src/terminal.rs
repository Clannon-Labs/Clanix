use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::{environment::Environment, podman};

pub(crate) async fn session(socket: WebSocket, environment: Arc<Environment>) {
    let mut child = match podman::spawn_terminal(environment.container_name()) {
        Ok(child) => child,
        Err(error) => {
            let (mut sender, _) = socket.split();
            let _ = sender
                .send(Message::Text(
                    format!("could not open terminal: {error}\n").into(),
                ))
                .await;
            environment.close_terminal();
            return;
        }
    };

    let mut stdin = child.stdin.take().expect("terminal stdin was piped");
    let stdout = child.stdout.take().expect("terminal stdout was piped");
    let stderr = child.stderr.take().expect("terminal stderr was piped");
    let (output_tx, mut output_rx) = mpsc::channel::<String>(32);
    tokio::spawn(read_output(stdout, output_tx.clone()));
    tokio::spawn(read_output(stderr, output_tx));

    let (mut sender, mut receiver) = socket.split();
    let _ = sender
        .send(Message::Text("Clannon environment ready.\n$ ".into()))
        .await;

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        environment.record_transcript("input", text.to_string()).await;
                        if stdin.write_all(text.as_bytes()).await.is_err() { break; }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        environment.record_transcript("input", text).await;
                        if stdin.write_all(&bytes).await.is_err() { break; }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        if sender.flush().await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) => {
                        let _ = sender.flush().await;
                        break;
                    }
                    None | Some(Err(_)) => break,
                    Some(Ok(Message::Pong(_))) => {}
                }
            }
            output = output_rx.recv() => {
                match output {
                    Some(text) => {
                        environment.record_transcript("output", text.clone()).await;
                        if sender.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    None => break,
                }
            }
        }
    }

    let _ = stdin.shutdown().await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    environment.close_terminal();
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
