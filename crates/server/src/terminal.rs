use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use runtime::{TerminalInput, TerminalReservation};

pub(crate) async fn session(socket: WebSocket, reservation: TerminalReservation) {
    let mut session = match reservation.open() {
        Ok(session) => session,
        Err(error) => {
            let (mut sender, _) = socket.split();
            let _ = sender
                .send(Message::Text(
                    format!("could not open terminal: {error}\n").into(),
                ))
                .await;
            return;
        }
    };

    let (mut sender, mut receiver) = socket.split();
    let _ = sender
        .send(Message::Text("Clannon environment ready.\n".into()))
        .await;

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if session.send(TerminalInput::Text(text.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if session.send(TerminalInput::Binary(bytes.to_vec())).await.is_err() {
                            break;
                        }
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
            output = session.next_output() => {
                match output {
                    Some(output) => {
                        let text = output.into_text();
                        if sender.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    None => break,
                }
            }
        }
    }

    session.close().await;
}
