use axum::{
    Router,
    body::Body,
    extract::{Path, State, WebSocketUpgrade},
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Serialize;

use runtime::{ObservationSnapshot, Runtime};

use crate::{
    access::{self, AccessPolicy},
    error::AppError,
    terminal,
};

#[derive(Clone)]
pub(crate) struct AppState {
    runtime: Runtime,
    access: AccessPolicy,
}

impl AppState {
    pub(crate) fn new(runtime: Runtime, access: AccessPolicy) -> Self {
        Self { runtime, access }
    }
}

#[derive(Serialize)]
struct EnvironmentCreated {
    id: String,
}

pub(crate) fn routes(state: AppState) -> Router {
    let access = state.access.clone();
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(javascript))
        .route("/styles.css", get(styles))
        .route("/api/access", get(access_check))
        .route("/api/environments", post(create_environment))
        .route("/api/environments/{id}", delete(destroy_environment))
        .route("/api/environments/{id}/observations", get(observations))
        .route("/api/environments/{id}/terminal", get(terminal))
        .with_state(state)
        .layer(middleware::from_fn(move |request, next| {
            access::enforce(access.clone(), request, next)
        }))
}

async fn access_check() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        StatusCode::NO_CONTENT,
    )
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

async fn create_environment(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let id = state.runtime.create().await?;
    Ok((StatusCode::CREATED, axum::Json(EnvironmentCreated { id })))
}

async fn destroy_environment(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.runtime.destroy(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn observations(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::Json<ObservationSnapshot>, AppError> {
    Ok(axum::Json(state.runtime.observe(&id).await?))
}

async fn terminal(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let requested_protocols: Vec<_> = websocket
        .requested_protocols()
        .filter_map(|value| value.to_str().ok())
        .collect();
    if !terminal::supports_protocol(&requested_protocols) {
        return Ok((
            StatusCode::BAD_REQUEST,
            "unsupported terminal WebSocket subprotocol",
        )
            .into_response());
    }
    let reservation = state.runtime.reserve_terminal(&id).await?;
    let websocket = websocket
        .max_message_size(terminal::MAX_WEBSOCKET_MESSAGE_BYTES)
        .max_frame_size(terminal::MAX_WEBSOCKET_MESSAGE_BYTES)
        .protocols([terminal::SUBPROTOCOL]);
    Ok(websocket.on_upgrade(move |socket| terminal::session(socket, reservation)))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use tower::ServiceExt;

    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn test_routes() -> Router {
        let runtime = Runtime::new("unused-in-router-tests".to_owned());
        let access = AccessPolicy::for_test("127.0.0.1:3000".parse().unwrap(), TOKEN);
        routes(AppState::new(runtime, access))
    }

    fn request(uri: &str, host: Option<&str>) -> axum::http::request::Builder {
        let builder = Request::builder().uri(uri);
        match host {
            Some(host) => builder.header(header::HOST, host),
            None => builder,
        }
    }

    #[tokio::test]
    async fn static_assets_require_only_an_allowed_host() {
        let response = test_routes()
            .oneshot(
                request("/", Some("localhost:3000"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap(),
            "frame-ancestors 'none'"
        );
        assert_eq!(
            response.headers().get(header::X_FRAME_OPTIONS).unwrap(),
            "DENY"
        );
    }

    #[tokio::test]
    async fn host_is_checked_before_origin_and_capability() {
        for host in [
            None,
            Some("clannon.example:3000"),
            Some("localhost.:3000"),
            Some("127.0.0.1:4000"),
        ] {
            let response = test_routes()
                .oneshot(
                    request("/api/environments/missing/observations", host)
                        .header(header::ORIGIN, "http://evil.example")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);
        }
    }

    #[tokio::test]
    async fn present_origin_is_checked_before_capability() {
        let response = test_routes()
            .oneshot(
                request(
                    "/api/environments/missing/observations",
                    Some("127.0.0.1:3000"),
                )
                .header(header::ORIGIN, "http://evil.example")
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn api_requires_the_exact_bearer_capability() {
        for authorization in [None, Some("Bearer wrong"), Some(TOKEN)] {
            let mut builder = request(
                "/api/environments/missing/observations",
                Some("127.0.0.1:3000"),
            )
            .header(header::ORIGIN, "http://127.0.0.1:3000");
            if let Some(authorization) = authorization {
                let value = if authorization == TOKEN {
                    format!("Bearer {authorization}")
                } else {
                    authorization.to_owned()
                };
                builder = builder.header(header::AUTHORIZATION, value);
            }
            let response = test_routes()
                .oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let expected = if authorization == Some(TOKEN) {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::UNAUTHORIZED
            };
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test]
    async fn valid_capability_reaches_nonexistent_environment_lookup_without_origin() {
        let response = test_routes()
            .oneshot(
                request(
                    "/api/environments/does-not-exist/observations",
                    Some("localhost:3000"),
                )
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn access_probe_is_never_cacheable() {
        let response = test_routes()
            .oneshot(
                request("/api/access", Some("localhost:3000"))
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }

    #[tokio::test]
    async fn terminal_uses_query_capability_instead_of_bearer() {
        let missing_query = test_routes()
            .oneshot(
                request("/api/environments/missing/terminal", Some("localhost:3000"))
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_query.status(), StatusCode::UNAUTHORIZED);

        let valid_query = test_routes()
            .oneshot(
                request(
                    &format!("/api/environments/missing/terminal?access_token={TOKEN}"),
                    Some("localhost:3000"),
                )
                .header(header::CONNECTION, "upgrade")
                .header(header::UPGRADE, "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .header("sec-websocket-protocol", terminal::SUBPROTOCOL)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        // The in-memory router has no Hyper upgrade extension, so reaching the
        // WebSocket extractor is represented by 426 rather than an upgrade.
        assert_eq!(valid_query.status(), StatusCode::UPGRADE_REQUIRED);

        for (origin, token, expected) in [
            ("http://evil.example", TOKEN, StatusCode::FORBIDDEN),
            ("http://localhost:3000", "wrong", StatusCode::UNAUTHORIZED),
        ] {
            let response = test_routes()
                .oneshot(
                    request(
                        &format!("/api/environments/missing/terminal?access_token={token}"),
                        Some("localhost:3000"),
                    )
                    .header(header::ORIGIN, origin)
                    .header(header::CONNECTION, "upgrade")
                    .header(header::UPGRADE, "websocket")
                    .header("sec-websocket-version", "13")
                    .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                    .header("sec-websocket-protocol", terminal::SUBPROTOCOL)
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
    }
}
