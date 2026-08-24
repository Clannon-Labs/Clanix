use axum::{
    Router,
    body::Body,
    extract::{Path, State, WebSocketUpgrade},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Serialize;

use crate::{environment::AppState, error::AppError, observation, terminal};

#[derive(Serialize)]
struct EnvironmentCreated {
    id: String,
}

pub(crate) fn routes(state: AppState) -> Router {
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
    let id = state.create().await?;
    Ok((StatusCode::CREATED, axum::Json(EnvironmentCreated { id })))
}

async fn destroy_environment(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.destroy(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn observations(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::Json<observation::ObservationSnapshot>, AppError> {
    let environment = state.find(&id).await?;
    Ok(axum::Json(observation::collect(environment).await))
}

async fn terminal(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let environment = state.find(&id).await?;
    if !environment.try_open_terminal() {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "a terminal is already connected",
        ));
    }
    Ok(websocket.on_upgrade(move |socket| terminal::session(socket, environment)))
}
