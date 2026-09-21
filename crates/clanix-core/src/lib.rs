
use std::borrow::Cow;

use std::{
    process::Command,
};
use uuid::Uuid;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    program: String,
    args: Vec<String>,
    policy: Policy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequestBuilder {
    program: String,
    args: Vec<String>,
    policy: Policy,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    id: Uuid,
    success: bool,
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    timeout_ms: u64,
}

impl Policy {
    fn default() -> Self {
        Self {
            timeout_ms: 100,
        }
    }
}

impl ExecutionRequest {
    pub fn new(program: String, args: Vec<String>) -> ExecutionRequestBuilder {
        ExecutionRequestBuilder {
            program: program,
            args: args,
            policy: Policy::default()
        }
    }   
}

impl ExecutionRequestBuilder {
    // It consumes the request after you call execute,
    // so you have to create a new request if you want
    // to call execute again
    pub fn execute(self) -> Result<ExecutionResult, String> {
        let Ok(result) = Command::new(&self.program)
            .args(&self.args)
            .output() else {
                return Err("Error occured while running command".to_string())
            };
        
        Ok(ExecutionResult {
            id: Uuid::new_v4(),
            success: result.status.success(),
            status: result.status.code(),
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }
    
    pub fn program(mut self, program: String) -> Self {
        self.program = program;
        self
    }

    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }
}

impl ExecutionResult {
    // These simple methods exist for the caller to call,
    // because the internal structure might change and
    // if the caller calls the internal elements directly,
    // it could break,
    // so calling these methods is safer because these will
    // translate whatever was changed internally into what the
    // callers expect; with no breaking changes
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    pub fn success(&self) -> bool {
        self.success
    }
    
    pub fn status(&self) -> Option<i32> {
        self.status
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stdout_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub fn stderr(&self) -> &[u8]  {
        &self.stderr
    }

    pub fn stderr_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    // Doing .clone() for these 2 functions for now
    // but i might have to figure out a way to avoid
    // this
    pub fn set_stdout(&mut self, new_stdout: String) {
        self.stdout = new_stdout.into_bytes()
    }

    pub fn set_stderr(&mut self, new_stderr: String) {
        self.stderr = new_stderr.into_bytes()
    }
}
