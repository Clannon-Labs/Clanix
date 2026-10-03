
use std::ffi::OsString;

use serde::Serialize;

use crate::{ExecutionRequest, ExecutionResult};
use crate::Policy;
use crate::ConfigBuilder;
use crate::{Error, Result}; // get the default Error type automatically

use crate::ResultError;

// This could be made richer and better instead of just a glue of other structs
#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    request: ExecutionRequest,
    result: Option<ExecutionResult>,
    config: ConfigBuilder,
}

impl Environment {
    pub fn new() -> Result<Self> {
        Ok(Self {
            request: ExecutionRequest::default()?,
            result: None,
            config: ConfigBuilder::default()?,
        })
    }

    pub fn new_request<S: Into<OsString>>(&self, program: S) -> Result<ExecutionRequest> {
        Ok(ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default()?,
        })
    }

    pub fn execute_request(mut self, request: ExecutionRequest) -> Result<Self> {
        let result = ExecutionRequest::execute(&request)?;
        self.request = request;
        self.result = Some(result);

        Ok(self)
    }
    
    pub fn new_result(&self) -> Result<ExecutionResult> {
        Ok(ExecutionResult::default())
    }
    
    pub fn what(&self) -> Result<&Self> {
        Ok(&self)
    }

    pub fn request(&self) -> Result<&ExecutionRequest> {
        Ok(&self.request)
    }

    pub fn result(self) -> Result<ExecutionResult> {
        self.result
            .ok_or_else(|| Error::Result(ResultError::Empty)) 
    }
    
    pub fn config(&self) -> Result<&ConfigBuilder> {
        Ok(&self.config)
    }

    pub fn policy(&self) -> Result<&Policy> {
        Ok(&self.request.policy)
    }

}
