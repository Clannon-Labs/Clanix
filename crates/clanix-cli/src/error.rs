
use thiserror::Error as ThisError;

use clanix::{
    ConfigError,
    PolicyError,
    RequestError,
    ResultError,

    Error as ClanixError,
};

// The consumers of this Error enum can import this Result<> type
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    #[error("core error: {0}")]
    Clanix(#[from] ClanixError),

    #[error("cli error: {0}")]
    Cli(#[from] CliError),
}

#[derive(Debug, ThisError)]
pub enum CliError {
    // #[error("invalid flag")]
    // InvalidFlag,

    // #[error("invalid program")]
    // InvalidProgram,

    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error")]
    Json(#[from] serde_json::Error),

    #[error("TOML serialization error")]
    Toml(#[from] toml::ser::Error),
}


impl From<RequestError> for Error {
    fn from(error: RequestError) -> Self {
        Self::Clanix(ClanixError::from(error))
    }
}

impl From<ResultError> for Error {
    fn from(error: ResultError) -> Self {
        Self::Clanix(ClanixError::from(error))
    }
}

impl From<PolicyError> for Error {
    fn from(error: PolicyError) -> Self {
        Self::Clanix(ClanixError::from(error))
    }
}

impl From<ConfigError> for Error {
    fn from(error: ConfigError) -> Self {
        Self::Clanix(ClanixError::from(error))
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Cli(CliError::Io(error))
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Cli(CliError::Json(error))
    }
}

impl From<toml::ser::Error> for Error {
    fn from(error: toml::ser::Error) -> Self {
        Self::Cli(CliError::Toml(error))
    }
}
