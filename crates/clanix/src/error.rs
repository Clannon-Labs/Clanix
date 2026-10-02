
use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    #[error("Request stage failed: {0}")]
    Request(#[from] RequestError),

    #[error("Result stage failed: {0}")]
    Result(#[from] ResultError),

    // to convert policy error back to request error
    #[error("Invalid execution policy")]
    Policy(#[from] PolicyError),

    // to convert policy error back to request error
    #[error("Invalid execution config")]
    Config(#[from] ConfigError),
    
    #[error("Unknown stage found!")]
    Unknown,
}


#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum RequestError {
    #[error("Couldnot spawn a child process")]
    SpawnError,

    #[error("Couldn't get stdout stream!")]
    StdoutError,

    #[error("Couldn't get stderr stream!")]
    StderrError,

    #[error("Couldn't get exit status from the process")]
    StatusError,
    
    #[error("Program is empty")]
    EmptyProgram,

    #[error("Argument(s) is invalid")]
    InvalidArgs,
}

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum ResultError {
    #[error("Invalid id found in the result")]
    InvalidId,

    #[error("Invalid success type")]
    InvalidSuccessType,

    #[error("Invalid status type")]
    InvalidStatus,

    #[error("Invalid stdout")]
    InvalidStdout,

    #[error("Invalid stderr")]
    InvalidStderr,

    #[error("Invalid working directory found!")]
    InvalidWorkingDir
}


#[derive(Debug, ThisError)]
#[non_exhaustive] // to ensure future addition in errors wont break the system
pub enum PolicyError {
    #[error("Invalid disable timeout type")]
    InvaildDisableTimeout,

    #[error("Invalid timeout")]
    InvalidTimeout,

    #[error("Invalid working directory")]
    InvalidWorkingDir,

    #[error("Error occured while reading config value")]
    Config(#[from] ConfigError)
}

#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("Couldn't get the default working directory for the current environment")]
    WorkingDir,
    
    #[error("Couldn't open the config file")]
    Open,
    
    #[error("Couldn't read value from config!")]
    Read,

    #[error("Couldn't write value to config!")]
    Write,

    #[error("Couldn't prepare config value!")]
    Prepare,

    #[error("Couldn't parse config value!")]
    Parse,
}

