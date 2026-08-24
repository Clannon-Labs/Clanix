use std::{
    collections::HashMap,
    net::SocketAddr,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{Mutex, mpsc},
};

const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_IMAGE: &str = "docker.io/library/alpine:3.20";
const MAX_TRANSCRIPT_ENTRIES: usize = 500;

#[derive(Clone)]
struct AppState {
    inner: Arc<StateInner>,
}

struct StateInner {
    environments: Mutex<HashMap<String, Arc<Environment>>>,
    next_id: AtomicU64,
    container_prefix: String,
    image: String,
}

struct Environment {
    id: String,
    container_name: String,
    transcript: Mutex<Vec<TranscriptEntry>>,
    terminal_active: AtomicBool,
}

#[derive(Clone, Serialize)]
struct TranscriptEntry {
    timestamp_ms: u128,
    direction: &'static str,
    data: String,
}

#[derive(Serialize)]
struct EnvironmentCreated {
    id: String,
}

#[derive(Serialize)]
struct ObservationSnapshot {
    environment_id: String,
    captured_at_ms: u128,
    transcript: Vec<TranscriptEntry>,
    processes: Vec<ProcessObservation>,
    files: Vec<FileObservation>,
    network: Vec<NetworkObservation>,
    warnings: Vec<String>,
}

#[derive(Debug, PartialEq, Serialize)]
struct ProcessObservation {
    pid: u32,
    parent_pid: u32,
    state: String,
    command: String,
    arguments: String,
}

#[derive(Debug, PartialEq, Serialize)]
struct FileObservation {
    path: String,
    size_bytes: u64,
    modified_unix_seconds: u64,
    kind: String,
}

#[derive(Debug, PartialEq, Serialize)]
struct NetworkObservation {
    protocol: String,
    local_address: String,
    remote_address: String,
    state: String,
}

struct AppError {
    status: StatusCode,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(context: &str, error: impl std::fmt::Display) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{context}: {error}"),
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() {
    let bind = std::env::var("CLANNON_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let image = std::env::var("CLANNON_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_owned());
    let address: SocketAddr = bind.parse().unwrap_or_else(|error| {
        eprintln!("invalid CLANNON_BIND {bind:?}: {error}");
        std::process::exit(2);
    });

    if let Err(error) = verify_podman().await {
        eprintln!("Clannon requires a working rootless Podman installation: {error}");
        std::process::exit(1);
    }

    let state = AppState::new(image);
    let app = routes(state.clone());
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .unwrap_or_else(|error| {
            eprintln!("could not listen on {address}: {error}");
            std::process::exit(1);
        });

    println!("Clannon is ready at http://{address}");
    if let Err(error) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_and_cleanup(state))
        .await
    {
        eprintln!("server error: {error}");
    }
}

impl AppState {
    fn new(image: String) -> Self {
        let started_at = now_ms();
        Self {
            inner: Arc::new(StateInner {
                environments: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                container_prefix: format!("clannon-{}-{started_at}", std::process::id()),
                image,
            }),
        }
    }

    async fn find(&self, id: &str) -> Result<Arc<Environment>, AppError> {
        self.inner
            .environments
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "environment not found"))
    }

    async fn cleanup(&self) {
        let environments: Vec<_> = self
            .inner
            .environments
            .lock()
            .await
            .drain()
            .map(|(_, env)| env)
            .collect();
        for environment in environments {
            let _ = remove_container(&environment.container_name).await;
        }
    }
}

fn routes(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(javascript))
        .route("/styles.css", get(styles))
        .route("/api/environments", post(create_environment))
        .route("/api/environments/{id}", delete(destroy_environment))
        .route("/api/environments/{id}/observations", get(observations))
        .route("/api/environments/{id}/terminal", get(terminal))
        .with_state(state)
}

async fn index() -> Response {
    asset(
        "text/html; charset=utf-8",
        include_str!("../static/index.html"),
    )
}

async fn javascript() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../static/app.js"),
    )
}

async fn styles() -> Response {
    asset(
        "text/css; charset=utf-8",
        include_str!("../static/styles.css"),
    )
}

fn asset(content_type: &'static str, source: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(source))
        .expect("static response is valid")
}

async fn create_environment(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let sequence = state.inner.next_id.fetch_add(1, Ordering::Relaxed);
    let id = format!("env-{sequence:08x}");
    let container_name = format!("{}-{sequence:08x}", state.inner.container_prefix);

    let output = Command::new("podman")
        .args([
            "run",
            "--detach",
            "--rm",
            "--name",
            &container_name,
            "--cap-drop=all",
            "--security-opt=no-new-privileges",
            "--pids-limit=256",
            "--memory=512m",
            "--cpus=1",
            "--tmpfs",
            "/workspace:rw,exec,nosuid,size=256m",
            "--workdir=/workspace",
            &state.inner.image,
            "/bin/sh",
            "-c",
            "trap 'exit 0' TERM INT; while :; do sleep 3600; done",
        ])
        .output()
        .await
        .map_err(|error| AppError::internal("could not start Podman", error))?;

    if !output.status.success() {
        return Err(command_error(
            "could not create environment",
            &output.stderr,
        ));
    }

    let environment = Arc::new(Environment {
        id: id.clone(),
        container_name,
        transcript: Mutex::new(Vec::new()),
        terminal_active: AtomicBool::new(false),
    });
    state
        .inner
        .environments
        .lock()
        .await
        .insert(id.clone(), environment);

    Ok((StatusCode::CREATED, axum::Json(EnvironmentCreated { id })))
}

async fn destroy_environment(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let environment = state
        .inner
        .environments
        .lock()
        .await
        .remove(&id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "environment not found"))?;

    match remove_container(&environment.container_name).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(error) => {
            // Keep ownership when Podman fails so the user can retry destruction
            // instead of leaving an unreachable container behind.
            state
                .inner
                .environments
                .lock()
                .await
                .insert(id, environment);
            Err(error)
        }
    }
}

async fn terminal(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let environment = state.find(&id).await?;
    if environment.terminal_active.swap(true, Ordering::AcqRel) {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "a terminal is already connected",
        ));
    }
    Ok(websocket.on_upgrade(move |socket| terminal_session(socket, environment)))
}

async fn terminal_session(socket: WebSocket, environment: Arc<Environment>) {
    let mut child = match Command::new("podman")
        .args([
            "exec",
            "-i",
            "--workdir",
            "/workspace",
            &environment.container_name,
            "/bin/sh",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let (mut sender, _) = socket.split();
            let _ = sender
                .send(Message::Text(
                    format!("could not open terminal: {error}\n").into(),
                ))
                .await;
            environment.terminal_active.store(false, Ordering::Release);
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
                        record_transcript(&environment, "input", text.to_string()).await;
                        if stdin.write_all(text.as_bytes()).await.is_err() { break; }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        record_transcript(&environment, "input", text).await;
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
                        record_transcript(&environment, "output", text.clone()).await;
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
    environment.terminal_active.store(false, Ordering::Release);
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

async fn record_transcript(environment: &Environment, direction: &'static str, data: String) {
    let mut transcript = environment.transcript.lock().await;
    transcript.push(TranscriptEntry {
        timestamp_ms: now_ms(),
        direction,
        data,
    });
    if transcript.len() > MAX_TRANSCRIPT_ENTRIES {
        let excess = transcript.len() - MAX_TRANSCRIPT_ENTRIES;
        transcript.drain(..excess);
    }
}

async fn observations(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::Json<ObservationSnapshot>, AppError> {
    let environment = state.find(&id).await?;
    let mut warnings = Vec::new();

    let processes =
        match exec_in_container(&environment.container_name, "ps -o pid,ppid,stat,comm,args").await
        {
            Ok(raw) => parse_processes(&raw),
            Err(error) => {
                warnings.push(error.message);
                Vec::new()
            }
        };
    let files = match exec_in_container(
        &environment.container_name,
        "find /workspace -mindepth 1 -maxdepth 4 -exec stat -c '%n|%s|%Y|%F' '{}' ';' 2>/dev/null | head -200",
    ).await {
        Ok(raw) => parse_files(&raw),
        Err(error) => { warnings.push(error.message); Vec::new() }
    };
    let network = match exec_in_container(
        &environment.container_name,
        "for f in tcp tcp6 udp udp6; do echo __$f__; cat /proc/net/$f 2>/dev/null; done",
    )
    .await
    {
        Ok(raw) => parse_network(&raw),
        Err(error) => {
            warnings.push(error.message);
            Vec::new()
        }
    };
    let transcript = environment.transcript.lock().await.clone();

    Ok(axum::Json(ObservationSnapshot {
        environment_id: environment.id.clone(),
        captured_at_ms: now_ms(),
        transcript,
        processes,
        files,
        network,
        warnings,
    }))
}

async fn exec_in_container(container: &str, script: &str) -> Result<String, AppError> {
    let output = Command::new("podman")
        .args(["exec", container, "/bin/sh", "-c", script])
        .output()
        .await
        .map_err(|error| AppError::internal("could not inspect environment", error))?;
    if !output.status.success() {
        return Err(command_error(
            "environment inspection failed",
            &output.stderr,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn remove_container(name: &str) -> Result<(), AppError> {
    let output = Command::new("podman")
        .args(["rm", "--force", "--ignore", name])
        .output()
        .await
        .map_err(|error| AppError::internal("could not destroy environment", error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            "could not destroy environment",
            &output.stderr,
        ))
    }
}

async fn verify_podman() -> Result<(), String> {
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

fn command_error(context: &str, stderr: &[u8]) -> AppError {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    AppError::new(
        StatusCode::BAD_GATEWAY,
        if detail.is_empty() {
            context.to_owned()
        } else {
            format!("{context}: {detail}")
        },
    )
}

fn parse_processes(raw: &str) -> Vec<ProcessObservation> {
    raw.lines()
        .skip_while(|line| line.trim_start().starts_with("PID"))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent_pid = fields.next()?.parse().ok()?;
            let state = fields.next()?.to_owned();
            let command = fields.next()?.to_owned();
            let arguments = fields.collect::<Vec<_>>().join(" ");
            Some(ProcessObservation {
                pid,
                parent_pid,
                state,
                command,
                arguments,
            })
        })
        .collect()
}

fn parse_files(raw: &str) -> Vec<FileObservation> {
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.rsplitn(4, '|');
            let kind = fields.next()?.to_owned();
            let modified_unix_seconds = fields.next()?.parse().ok()?;
            let size_bytes = fields.next()?.parse().ok()?;
            let path = fields.next()?.to_owned();
            Some(FileObservation {
                path,
                size_bytes,
                modified_unix_seconds,
                kind,
            })
        })
        .collect()
}

fn parse_network(raw: &str) -> Vec<NetworkObservation> {
    let mut protocol = String::new();
    let mut observations = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(marker) = trimmed
            .strip_prefix("__")
            .and_then(|value| value.strip_suffix("__"))
        {
            protocol = marker.to_owned();
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("sl") {
            continue;
        }
        let fields: Vec<_> = trimmed.split_whitespace().collect();
        if fields.len() >= 4 {
            observations.push(NetworkObservation {
                protocol: protocol.clone(),
                local_address: decode_endpoint(fields[1], protocol.ends_with('6')),
                remote_address: decode_endpoint(fields[2], protocol.ends_with('6')),
                state: network_state(fields[3]).to_owned(),
            });
        }
    }
    observations
}

fn decode_endpoint(raw: &str, ipv6: bool) -> String {
    let Some((address, port)) = raw.split_once(':') else {
        return raw.to_owned();
    };
    let Ok(port) = u16::from_str_radix(port, 16) else {
        return raw.to_owned();
    };
    if ipv6 {
        return format!("[{address}]:{port}");
    }
    let Ok(value) = u32::from_str_radix(address, 16) else {
        return raw.to_owned();
    };
    let octets = value.to_le_bytes();
    format!(
        "{}.{}.{}.{}:{port}",
        octets[0], octets[1], octets[2], octets[3]
    )
}

fn network_state(code: &str) -> &str {
    match code {
        "01" => "established",
        "02" => "syn-sent",
        "03" => "syn-received",
        "04" => "fin-wait-1",
        "05" => "fin-wait-2",
        "06" => "time-wait",
        "07" => "closed",
        "08" => "close-wait",
        "09" => "last-ack",
        "0A" => "listening",
        "0B" => "closing",
        _ => code,
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

async fn shutdown_and_cleanup(state: AppState) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    state.cleanup().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_busybox_process_output() {
        let raw = "PID   PPID  STAT COMMAND          COMMAND\n    1      0 S    sh               /bin/sh -c sleep 3600\n   12      1 R    demo             ./demo --verbose\n";
        assert_eq!(
            parse_processes(raw),
            vec![
                ProcessObservation {
                    pid: 1,
                    parent_pid: 0,
                    state: "S".into(),
                    command: "sh".into(),
                    arguments: "/bin/sh -c sleep 3600".into()
                },
                ProcessObservation {
                    pid: 12,
                    parent_pid: 1,
                    state: "R".into(),
                    command: "demo".into(),
                    arguments: "./demo --verbose".into()
                },
            ]
        );
    }

    #[test]
    fn parses_file_paths_containing_pipes() {
        let raw = "/workspace/a|b.txt|42|1720000000|regular file\n";
        assert_eq!(
            parse_files(raw),
            vec![FileObservation {
                path: "/workspace/a|b.txt".into(),
                size_bytes: 42,
                modified_unix_seconds: 1_720_000_000,
                kind: "regular file".into(),
            }]
        );
    }

    #[test]
    fn parses_and_decodes_network_snapshot() {
        let raw = "__tcp__\n  sl  local_address rem_address   st\n   0: 0100007F:1F90 00000000:0000 0A\n__udp__\n   2: 00000000:0035 00000000:0000 07\n";
        assert_eq!(
            parse_network(raw),
            vec![
                NetworkObservation {
                    protocol: "tcp".into(),
                    local_address: "127.0.0.1:8080".into(),
                    remote_address: "0.0.0.0:0".into(),
                    state: "listening".into()
                },
                NetworkObservation {
                    protocol: "udp".into(),
                    local_address: "0.0.0.0:53".into(),
                    remote_address: "0.0.0.0:0".into(),
                    state: "closed".into()
                },
            ]
        );
    }

    #[test]
    fn caps_transcript_without_losing_newest_entries() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let environment = Environment {
                id: "test".into(),
                container_name: "none".into(),
                transcript: Mutex::new(Vec::new()),
                terminal_active: AtomicBool::new(false),
            };
            for index in 0..=MAX_TRANSCRIPT_ENTRIES {
                record_transcript(&environment, "output", index.to_string()).await;
            }
            let transcript = environment.transcript.lock().await;
            assert_eq!(transcript.len(), MAX_TRANSCRIPT_ENTRIES);
            assert_eq!(transcript.first().unwrap().data, "1");
            assert_eq!(
                transcript.last().unwrap().data,
                MAX_TRANSCRIPT_ENTRIES.to_string()
            );
        });
    }
}
