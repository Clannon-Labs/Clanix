
use std::ffi::OsString;

use serde::Serialize;

use crate::{ExecutionRequest, ExecutionResult};
use crate::Policy;
// use crate::ConfigBuilder;
use crate::{Error, Result}; // get the default Error type automatically

// This could be made richer and better instead of just a glue of other structs
#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    request: ExecutionRequest,
    result: Option<ExecutionResult>
}

impl Environment {
    pub fn new() -> Result<Self> {
        Ok(Self {
            request: ExecutionRequest::default()?,
            result: None
        })
    }

    pub fn request<S: Into<OsString>>(program: S) -> Result<ExecutionRequest> {
        Ok(ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::defaults()?
        })
    }

    pub fn new_result() -> Result<ExecutionResult> {
        ExecutionResult::defaults()
            .map_err(Error::from)
    }
}
