
//! In the clanix crate, this is the entry point for its consumers to use clanix
//! They can't directly access anything inside clanix directly

use std::ffi::OsString;

use serde::{Serialize, Deserialize};

use crate::{ExecutionRequest, ExecutionResult};
use crate::Policy;

// Just a proposed idea for now, no real implementation yet

// This could be made richer and better instead of just a glue of other structs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEnvironment {
    request: ExecutionRequest,
    // policy: Policy, // Policy is already in the request, so we dont need to replicate it
    result: Option<ExecutionResult>
}

impl ExecutionEnvironment {
    pub fn new() -> Self {
        Self {
            request: ExecutionRequest::default(),
            result: None
        }
    }

    pub fn request<S: Into<OsString>>(program: S) -> ExecutionRequest {
        ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::defaults()
        }
    }

    pub fn result() -> ExecutionResult {
        ExecutionResult::defaults()
    }
}