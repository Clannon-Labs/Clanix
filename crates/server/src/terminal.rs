use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use runtime::{TerminalDimensions, TerminalEvent, TerminalInput, TerminalReservation};
use serde::{Deserialize, Serialize};

pub(crate) const SUBPROTOCOL: &str = "clannon.terminal.v1";
const VERSION: u8 = 1;
const MAX_INPUT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_WEBSOCKET_MESSAGE_BYTES: usize = MAX_INPUT_BYTES * 6 + 1024;
// Runtime PTY setup owns a four-second handshake plus bounded helper cleanup.
// Keep this outer protocol deadline longer so cancellation cannot cut cleanup short.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const FINAL_EVENT_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_NORMAL: u16 = 1000;
const CLOSE_PROTOCOL: u16 = 1002;
const CLOSE_RUNTIME: u16 = 1011;
pub(crate) fn supports_protocol(requested: &[&str]) -> bool {
    requested.contains(&SUBPROTOCOL)
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", deny_unknown_fields)]
enum ClientControl {
    #[serde(rename = "open")]
    Open {
        version: u8,
        columns: u16,
        rows: u16,
    },
    #[serde(rename = "input")]
    Input { data: String },
    #[serde(rename = "resize")]
    Resize { columns: u16, rows: u16 },
}

#[derive(Debug, PartialEq)]
struct OpenRequest {
    columns: u16,
    rows: u16,
}

#[derive(Debug, PartialEq)]
enum InputAction {
    Send(TerminalInputData),
    Resize { columns: u16, rows: u16 },
    Ignore,
    Close,
}

#[derive(Debug, PartialEq)]
enum TerminalInputData {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
enum ServerControl<'a> {
    #[serde(rename = "ready")]
    Ready {
        version: u8,
        resumed: bool,
        resize: bool,
    },
    #[serde(rename = "exit")]
    Exit { code: Option<i32> },
    #[serde(rename = "error")]
    Error { code: &'a str, message: &'a str },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CloseKind {
    Normal,
    Protocol,
    Runtime,
}

impl CloseKind {
    fn code(self) -> u16 {
        match self {
            Self::Normal => CLOSE_NORMAL,
            Self::Protocol => CLOSE_PROTOCOL,
            Self::Runtime => CLOSE_RUNTIME,
        }
    }
}

pub(crate) async fn session(socket: WebSocket, reservation: TerminalReservation) {
    let (mut sender, mut receiver) = socket.split();
    let open = match tokio::time::timeout(OPEN_TIMEOUT, receive_open(&mut receiver)).await {
        Ok(Ok(open)) => open,
        Ok(Err(message)) => {
            finish_with_error(&mut sender, "protocol_error", &message, CloseKind::Protocol).await;
            return;
        }
        Err(_) => {
            finish_with_error(
                &mut sender,
                "protocol_error",
                "timed out waiting for terminal open control",
                CloseKind::Protocol,
            )
            .await;
            return;
        }
    };
    let initial_dimensions = match terminal_dimensions(open.columns, open.rows) {
        Ok(dimensions) => dimensions,
        Err(message) => {
            finish_with_error(&mut sender, "protocol_error", &message, CloseKind::Protocol).await;
            return;
        }
    };

    let mut attachment =
        match tokio::time::timeout(OPEN_TIMEOUT, reservation.open(initial_dimensions)).await {
            Ok(Ok(attachment)) => attachment,
            Ok(Err(error)) => {
                finish_with_error(
                    &mut sender,
                    "runtime_error",
                    &format!("could not open terminal: {error}"),
                    CloseKind::Runtime,
                )
                .await;
                return;
            }
            Err(_) => {
                finish_with_error(
                    &mut sender,
                    "runtime_error",
                    "timed out opening terminal",
                    CloseKind::Runtime,
                )
                .await;
                return;
            }
        };
    if send_control(
        &mut sender,
        &ServerControl::Ready {
            version: VERSION,
            resumed: attachment.resumed(),
            resize: true,
        },
    )
    .await
    .is_err()
    {
        attachment.close().await;
        return;
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(message)) => match parse_input(message) {
                        Ok(InputAction::Send(input)) => {
                            let input = match input {
                                TerminalInputData::Text(text) => TerminalInput::Text(text),
                                TerminalInputData::Binary(bytes) => TerminalInput::Binary(bytes),
                            };
                            if let Err(error) = attachment.send(input).await {
                                finish_after_write_error(&mut sender, &mut attachment, &error.to_string()).await;
                                break;
                            }
                        }
                        Ok(InputAction::Resize { columns, rows }) => {
                            let dimensions = match terminal_dimensions(columns, rows) {
                                Ok(dimensions) => dimensions,
                                Err(message) => {
                                    finish_with_error(
                                        &mut sender,
                                        "protocol_error",
                                        &message,
                                        CloseKind::Protocol,
                                    ).await;
                                    break;
                                }
                            };
                            if let Err(error) = attachment.resize(dimensions).await {
                                finish_with_error(
                                    &mut sender,
                                    "runtime_error",
                                    &format!("could not resize terminal: {error}"),
                                    CloseKind::Runtime,
                                ).await;
                                break;
                            }
                        }
                        Ok(InputAction::Ignore) => {}
                        Ok(InputAction::Close) => break,
                        Err(message) => {
                            finish_with_error(
                                &mut sender,
                                "protocol_error",
                                &message,
                                CloseKind::Protocol,
                            ).await;
                            break;
                        }
                    },
                    Some(Err(error)) => {
                        finish_with_error(
                            &mut sender,
                            "protocol_error",
                            &format!("invalid terminal WebSocket frame: {error}"),
                            CloseKind::Protocol,
                        ).await;
                        break;
                    }
                    None => break,
                }
            }
            event = attachment.next_event() => {
                match event {
                    Some(TerminalEvent::Output(bytes)) => {
                        if sender.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(TerminalEvent::OutputGap { missed }) => {
                        finish_with_error(
                            &mut sender,
                            "output_gap",
                            &format!("terminal output fell behind by {missed} events; reconnect to the preserved shell"),
                            CloseKind::Runtime,
                        ).await;
                        break;
                    }
                    Some(TerminalEvent::Exited { code }) => {
                        let _ = send_control(&mut sender, &ServerControl::Exit { code }).await;
                        let _ = send_close(&mut sender, CloseKind::Normal, "terminal exited").await;
                        break;
                    }
                    Some(TerminalEvent::Failed(message)) => {
                        finish_with_error(
                            &mut sender,
                            "runtime_error",
                            &message,
                            CloseKind::Runtime,
                        ).await;
                        break;
                    }
                    None => {
                        finish_with_error(
                            &mut sender,
                            "runtime_error",
                            "terminal event stream ended unexpectedly",
                            CloseKind::Runtime,
                        ).await;
                        break;
                    }
                }
            }
        }
    }

    attachment.close().await;
}

async fn finish_after_write_error(
    sender: &mut SplitSink<WebSocket, Message>,
    attachment: &mut runtime::TerminalAttachment,
    write_error: &str,
) {
    let ending = tokio::time::timeout(FINAL_EVENT_TIMEOUT, async {
        loop {
            match attachment.next_event().await {
                Some(TerminalEvent::Output(bytes)) => {
                    if sender.send(Message::Binary(bytes.into())).await.is_err() {
                        return None;
                    }
                }
                Some(TerminalEvent::OutputGap { .. }) => {}
                ending @ Some(TerminalEvent::Exited { .. } | TerminalEvent::Failed(_)) => {
                    return ending;
                }
                None => return None,
            }
        }
    })
    .await;

    match ending {
        Ok(Some(TerminalEvent::Exited { code })) => {
            let _ = send_control(sender, &ServerControl::Exit { code }).await;
            let _ = send_close(sender, CloseKind::Normal, "terminal exited").await;
        }
        Ok(Some(TerminalEvent::Failed(message))) => {
            finish_with_error(sender, "runtime_error", &message, CloseKind::Runtime).await;
        }
        Ok(Some(TerminalEvent::Output(_) | TerminalEvent::OutputGap { .. })) => unreachable!(),
        Ok(None) | Err(_) => {
            finish_with_error(
                sender,
                "runtime_error",
                &format!("could not write terminal input: {write_error}"),
                CloseKind::Runtime,
            )
            .await;
        }
    }
}

async fn receive_open(receiver: &mut SplitStream<WebSocket>) -> Result<OpenRequest, String> {
    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => return parse_open(&text),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None => {
                return Err("terminal closed before the open control".to_owned());
            }
            Some(Ok(Message::Binary(_))) => {
                return Err("the first terminal frame must be an open text control".to_owned());
            }
            Some(Err(error)) => return Err(format!("invalid terminal WebSocket frame: {error}")),
        }
    }
}

fn parse_open(text: &str) -> Result<OpenRequest, String> {
    let control = parse_control(text)?;
    let ClientControl::Open {
        version,
        columns,
        rows,
    } = control
    else {
        return Err("the first terminal control must be open".to_owned());
    };
    if version != VERSION {
        return Err(format!("unsupported terminal protocol version {version}"));
    }
    validate_dimensions(columns, rows)?;
    Ok(OpenRequest { columns, rows })
}

fn parse_input(message: Message) -> Result<InputAction, String> {
    match message {
        Message::Text(text) => {
            let control = parse_control(&text)?;
            match control {
                ClientControl::Input { data } => {
                    ensure_input_bound(data.len())?;
                    Ok(InputAction::Send(TerminalInputData::Text(data)))
                }
                ClientControl::Resize { columns, rows } => {
                    validate_dimensions(columns, rows)?;
                    Ok(InputAction::Resize { columns, rows })
                }
                ClientControl::Open { .. } => {
                    Err("only input or resize controls are valid after terminal ready".to_owned())
                }
            }
        }
        Message::Binary(bytes) => {
            ensure_input_bound(bytes.len())?;
            Ok(InputAction::Send(TerminalInputData::Binary(bytes.to_vec())))
        }
        Message::Close(_) => Ok(InputAction::Close),
        Message::Ping(_) | Message::Pong(_) => Ok(InputAction::Ignore),
    }
}

fn parse_control(text: &str) -> Result<ClientControl, String> {
    serde_json::from_str(text).map_err(|_| "malformed or unknown terminal control".to_owned())
}

fn ensure_input_bound(length: usize) -> Result<(), String> {
    if length > MAX_INPUT_BYTES {
        Err(format!(
            "terminal input exceeds the {MAX_INPUT_BYTES}-byte limit"
        ))
    } else {
        Ok(())
    }
}

fn validate_dimensions(columns: u16, rows: u16) -> Result<(), String> {
    if !(1..=1000).contains(&columns) || !(1..=1000).contains(&rows) {
        Err("terminal columns and rows must each be between 1 and 1000".to_owned())
    } else {
        Ok(())
    }
}

fn terminal_dimensions(columns: u16, rows: u16) -> Result<TerminalDimensions, String> {
    validate_dimensions(columns, rows)?;
    TerminalDimensions::new(columns, rows)
        .map_err(|_| "terminal columns and rows must each be between 1 and 1000".to_owned())
}

async fn finish_with_error(
    sender: &mut SplitSink<WebSocket, Message>,
    code: &str,
    message: &str,
    close: CloseKind,
) {
    let _ = send_control(sender, &ServerControl::Error { code, message }).await;
    let _ = send_close(sender, close, message).await;
}

async fn send_control(
    sender: &mut SplitSink<WebSocket, Message>,
    control: &ServerControl<'_>,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(control).expect("terminal controls are serializable");
    sender.send(Message::Text(text.into())).await
}

async fn send_close(
    sender: &mut SplitSink<WebSocket, Message>,
    kind: CloseKind,
    reason: &str,
) -> Result<(), axum::Error> {
    sender
        .send(Message::Close(Some(CloseFrame {
            code: kind.code(),
            reason: truncate_close_reason(reason).into(),
        })))
        .await
}

fn truncate_close_reason(reason: &str) -> &str {
    const MAX_REASON_BYTES: usize = 123;
    if reason.len() <= MAX_REASON_BYTES {
        return reason;
    }
    let mut end = MAX_REASON_BYTES;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    &reason[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_the_v1_subprotocol() {
        assert!(supports_protocol(&[SUBPROTOCOL]));
        assert!(supports_protocol(&["other", SUBPROTOCOL]));
        assert!(!supports_protocol(&[]));
        assert!(!supports_protocol(&["unknown"]));
    }

    #[test]
    fn parses_v1_open_and_rejects_wrong_shape_version_and_bounds() {
        assert_eq!(
            parse_open(r#"{"type":"open","version":1,"columns":80,"rows":24}"#),
            Ok(OpenRequest {
                columns: 80,
                rows: 24,
            })
        );
        for invalid in [
            r#"{"type":"input","data":"pwd"}"#,
            r#"{"type":"open","version":2,"columns":80,"rows":24}"#,
            r#"{"type":"open","version":1,"columns":0,"rows":24}"#,
            r#"{"type":"open","version":1,"columns":80,"rows":1001}"#,
            r#"{"type":"open","version":1,"columns":80,"rows":24,"extra":true}"#,
            "not json",
        ] {
            assert!(parse_open(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn parses_bounded_text_and_preserves_binary_bytes() {
        assert_eq!(
            parse_input(Message::Text(
                r#"{"type":"input","data":"echo hi\n"}"#.into()
            )),
            Ok(InputAction::Send(TerminalInputData::Text(
                "echo hi\n".to_owned()
            )))
        );
        assert_eq!(
            parse_input(Message::Binary(vec![0, 3, 0xff].into())),
            Ok(InputAction::Send(TerminalInputData::Binary(vec![
                0, 3, 0xff
            ])))
        );
        assert!(ensure_input_bound(MAX_INPUT_BYTES).is_ok());
        assert!(ensure_input_bound(MAX_INPUT_BYTES + 1).is_err());
        let oversized = format!(
            r#"{{"type":"input","data":"{}"}}"#,
            "x".repeat(MAX_INPUT_BYTES + 1)
        );
        assert!(parse_input(Message::Text(oversized.into())).is_err());
    }

    #[test]
    fn parses_exact_bounded_resize_only_after_ready() {
        assert_eq!(
            parse_input(Message::Text(
                r#"{"type":"resize","columns":120,"rows":36}"#.into()
            )),
            Ok(InputAction::Resize {
                columns: 120,
                rows: 36,
            })
        );
        assert!(
            parse_open(r#"{"type":"resize","columns":120,"rows":36}"#).is_err(),
            "resize must not be accepted before ready"
        );

        for invalid in [
            r#"{"type":"open","version":1,"columns":80,"rows":24}"#,
            r#"{"type":"resize","columns":0,"rows":36}"#,
            r#"{"type":"resize","columns":120,"rows":1001}"#,
            r#"{"type":"resize","columns":120}"#,
            r#"{"type":"resize","columns":"120","rows":36}"#,
            r#"{"type":"resize","columns":120,"rows":36,"extra":true}"#,
            r#"{"type":"unknown","columns":120,"rows":36}"#,
        ] {
            assert!(
                parse_input(Message::Text(invalid.into())).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn serializes_ready_with_resumed_and_resize_contract() {
        let fresh = serde_json::to_value(ServerControl::Ready {
            version: VERSION,
            resumed: false,
            resize: true,
        })
        .unwrap();
        let resumed = serde_json::to_value(ServerControl::Ready {
            version: VERSION,
            resumed: true,
            resize: true,
        })
        .unwrap();
        assert_eq!(
            fresh,
            serde_json::json!({"type":"ready","version":1,"resumed":false,"resize":true})
        );
        assert_eq!(
            resumed,
            serde_json::json!({"type":"ready","version":1,"resumed":true,"resize":true})
        );
    }

    #[test]
    fn serializes_exit_and_failure_controls() {
        assert_eq!(
            serde_json::to_value(ServerControl::Exit { code: Some(7) }).unwrap(),
            serde_json::json!({"type":"exit","code":7})
        );
        assert_eq!(
            serde_json::to_value(ServerControl::Exit { code: None }).unwrap(),
            serde_json::json!({"type":"exit","code":null})
        );
        assert_eq!(
            serde_json::to_value(ServerControl::Error {
                code: "runtime_error",
                message: "write failed",
            })
            .unwrap(),
            serde_json::json!({"type":"error","code":"runtime_error","message":"write failed"})
        );
    }

    #[test]
    fn maps_terminal_endings_to_websocket_close_codes() {
        assert_eq!(CloseKind::Normal.code(), 1000);
        assert_eq!(CloseKind::Protocol.code(), 1002);
        assert_eq!(CloseKind::Runtime.code(), 1011);
        let long = format!("{}é", "x".repeat(122));
        let truncated = truncate_close_reason(&long);
        assert!(truncated.len() <= 123);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
