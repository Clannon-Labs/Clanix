use axum::{
    Router,
    body::Body,
    extract::{Path, State, WebSocketUpgrade},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Serialize;

use runtime::{ObservationSnapshot, Runtime};

use crate::{error::AppError, terminal};

#[derive(Serialize)]
struct EnvironmentCreated {
    id: String,
}

pub(crate) fn routes(state: Runtime) -> Router {
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
        include_str!("../../../static/index.html"),
    )
}

async fn javascript() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../../../static/app.js"),
    )
}

async fn styles() -> Response {
    asset(
        "text/css; charset=utf-8",
        include_str!("../../../static/styles.css"),
    )
}

fn asset(content_type: &'static str, source: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(source))
        .expect("static response is valid")
}

async fn create_environment(State(state): State<Runtime>) -> Result<impl IntoResponse, AppError> {
    let id = state.create().await?;
    Ok((StatusCode::CREATED, axum::Json(EnvironmentCreated { id })))
}

async fn destroy_environment(
    State(state): State<Runtime>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.destroy(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn observations(
    State(state): State<Runtime>,
    Path(id): Path<String>,
) -> Result<axum::Json<ObservationSnapshot>, AppError> {
    Ok(axum::Json(state.observe(&id).await?))
}

async fn terminal(
    websocket: WebSocketUpgrade,
    State(state): State<Runtime>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let reservation = state.reserve_terminal(&id).await?;
    Ok(websocket.on_upgrade(move |socket| terminal::session(socket, reservation)))
}
