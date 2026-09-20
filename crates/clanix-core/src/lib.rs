
use std::{
    fs,
    process::{
        Command,
        Output,
    },
    path::PathBuf,
};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub program: String,
    pub args: Vec<String>
}

impl ExecutionRequest {
    pub fn execute(&self) -> Result<Output, std::io::Error> {
        Command::new(&self.program)
            .args(&self.args)
            .output()
    }
}
