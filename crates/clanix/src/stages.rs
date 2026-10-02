

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExecutionStage {
    Request,
    Result, // add others in future
    Unknown,
}


impl std::fmt::Display for ExecutionStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ExecutionStage::Request => "request",
            ExecutionStage::Result => "result",
            ExecutionStage::Unknown => "unknown",
        })
    }
}

