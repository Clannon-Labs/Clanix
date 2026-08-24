use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeErrorKind {
    NotFound,
    Conflict,
    BadGateway,
    Internal,
}

#[derive(Debug)]
pub struct RuntimeError {
    kind: RuntimeErrorKind,
    message: String,
}

impl RuntimeError {
    pub(crate) fn new(kind: RuntimeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn internal(context: &str, error: impl fmt::Display) -> Self {
        Self::new(RuntimeErrorKind::Internal, format!("{context}: {error}"))
    }

    pub fn kind(&self) -> RuntimeErrorKind {
        self.kind
    }

    pub fn into_message(self) -> String {
        self.message
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeError {}
