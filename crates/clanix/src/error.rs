//! The place where the error returned by the crate are defined
//! 
//! If you want to know what are the errors you might get as the consumer
//! of this crate, you just have to look in this file, and you would know 
//! everything related to the errors

use std::{ffi::OsString, path::PathBuf};
use std::time::{Duration};

use thiserror::Error as ThisError;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    #[error("request stage failed: {0}")]
    Request(#[from] RequestError),

    #[error("result stage failed: {0}")]
    Result(#[from] ResultError),

    #[error("project detection failed: {0}")]
    Detect(#[from] std::io::Error),

    #[error("failed to walk the directories: {0}")]
    WalkError(#[from] walkdir::Error),

    #[error("invalid filename {0}")]
    InvalidFilename(PathBuf),

    #[error("invalid directory name {0}")]
    InvalidDirectoryName(PathBuf),
    
    // Convert policy errors into the crate-level error type
    #[error("invalid execution policy: {0}")]
    Policy(#[from] PolicyError),

    // Convert config errors into the crate-level error type
    #[error("invalid execution config: {0}")]
    Config(#[from] ConfigError),

    #[error("couldn't convert to json: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("couldn't convert to toml: {0}")]
    TomlError(#[from] toml::ser::Error),
    
    #[error("unknown stage found!")]
    Unknown,
}


#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum RequestError {
    #[error("given request is not confirmed for execution. Call .confirm() to confirm it!")]
    ConfirmError,
    
    #[error(
        "couldn't spawn program {program:?} in the working directory {}: {source}", working_dir.display()
    )]
    SpawnError{
        program: OsString,
        working_dir: PathBuf,

        #[source]
        source: std::io::Error
    },

    #[error("child process did not provide a stdout stream!")]
    StdoutUnavailable,

    #[error("child process did not provide a stderr stream!")]
    StderrUnavailable,

    #[error("failed to read child process stdout: {0}")]
    StdoutRead(#[source] std::io::Error),
    
    #[error("failed to read child process stderr: {0}")]
    StderrRead(#[source] std::io::Error),

    #[error("failed to kill timed-out process: {0}")]
    Kill(#[source] std::io::Error),

    #[error(
        "Process '{0}..' ran for {elapsed_time:?} while the timeout was {timeout:?}",
    &id.to_string()[..10])]
    Timeout {
        id: Uuid,
        elapsed_time: Duration,
        timeout: Duration,
        working_dir: PathBuf,
    },
    
    #[error("couldn't get exit status from the process: {0}")]
    StatusError(#[source] std::io::Error),
    
    #[error("program is empty")]
    EmptyProgram,

    #[error("argument(s) is invalid: {0}")]
    InvalidArgs(String),

    #[error("couldn't serialize execution request as JSON: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("couldn't serialize execution result as TOML: {0}")]
    TomlError(#[from] toml::ser::Error),
}

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum ResultError {
    #[error("given result is empty")]
    Empty,

    #[error("couldn't convert to json: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("couldn't convert to toml: {0}")]
    TomlError(#[from] toml::ser::Error),
    
    #[error("invalid id found in the result")]
    InvalidId,

    #[error("invalid success type")]
    InvalidSuccessType,

    #[error("invalid status type")]
    InvalidStatus,

    #[error("invalid stdout")]
    InvalidStdout,

    #[error("invalid stderr")]
    InvalidStderr,

    #[error("invalid working directory found!")]
    InvalidWorkingDir
}


#[derive(Debug, ThisError)]
#[non_exhaustive] // to ensure future addition in errors wont break the system
pub enum PolicyError {
    #[error("invalid working directory: {}", path.display())]
    InvalidWorkingDir{
        path: PathBuf
    },

    #[error("error occured while reading config value: {0}")]
    Config(#[from] ConfigError)
}

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("couldn't get the default working directory: {0}")]
    WorkingDir(#[source] std::io::Error),
    
    #[error("couldn't read value from configuration file {}: {source}", path.display())]
    Read {
        path: PathBuf,

        #[source]
        source: std::io::Error
    },

    #[error("couldn't write value to configuration file {}: {source}", path.display())]
    Write {
        path: PathBuf,

        #[source]
        source: std::io::Error,
    },

    #[error("couldn't prepare configuration value as toml!: {0}")]
    PrepareToml(#[from] toml::ser::Error),

    #[error("couldn't parse configuration value as toml!: {0}")]
    ParseToml(#[from] toml::de::Error),

    #[error("couldn't serialize configuration as JSON: {0}")]
    JsonError(#[from] serde_json::Error),
}

