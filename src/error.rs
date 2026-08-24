use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub(crate) struct AppError {
    status: StatusCode,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl AppError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub(crate) fn internal(context: &str, error: impl std::fmt::Display) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{context}: {error}"),
        )
    }

    pub(crate) fn into_message(self) -> String {
        self.message
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
