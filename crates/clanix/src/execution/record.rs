use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{Effective, ExecutionRequest, ExecutionResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub(crate) request: ExecutionRequest<'static>,
    pub(crate) effective: Effective,
    pub(crate) started_at: SystemTime,
    pub(crate) result: ExecutionResult,
}

impl ExecutionRecord {
    pub fn record(
        request: ExecutionRequest<'static>,
        effective: Effective,
        started_at: SystemTime,
        result: &ExecutionResult,
    ) -> () {
        if let Some(env) = request.env {
            env.history.borrow_mut().push(Self {
                request,
                effective,
                started_at,
                result: result.clone(),
            });
        }

        // We are holding quite a bit of data here, so I dont think we should
        // return all of this again to the caller as they already gave us the pieces
        // which we just assembled and pushed; that's it
        // They already have all the data
        ()
    }

    pub fn request(&self) -> &ExecutionRequest<'static> {
        &self.request
    }

    pub fn result(&self) -> &ExecutionResult {
        &self.result
    }

    pub fn effective(&self) -> &Effective {
        &self.effective
    }
}
