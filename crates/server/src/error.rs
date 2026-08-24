use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use runtime::{RuntimeError, RuntimeErrorKind};
use serde::Serialize;

pub(crate) struct AppError(RuntimeError);

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl From<RuntimeError> for AppError {
    fn from(error: RuntimeError) -> Self {
        Self(error)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self.0.kind() {
            RuntimeErrorKind::NotFound => StatusCode::NOT_FOUND,
            RuntimeErrorKind::Conflict => StatusCode::CONFLICT,
            RuntimeErrorKind::BadGateway => StatusCode::BAD_GATEWAY,
            RuntimeErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            axum::Json(ErrorBody {
                error: self.0.into_message(),
            }),
        )
            .into_response()
    }
}
