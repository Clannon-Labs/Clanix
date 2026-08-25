use serde::Serialize;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct ProcessObservation {
    pub(crate) pid: u32,
    pub(crate) parent_pid: u32,
    pub(crate) state: String,
    pub(crate) command: String,
    pub(crate) arguments: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct FileObservation {
    pub(crate) path: String,
    pub(crate) size_bytes: u64,
    pub(crate) modified_unix_seconds: u64,
    pub(crate) kind: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct NetworkObservation {
    pub(crate) protocol: String,
    pub(crate) local_address: String,
    pub(crate) remote_address: String,
    pub(crate) state: String,
}

impl ProcessObservation {
    pub(crate) fn estimated_owned_bytes(&self) -> usize {
        self.state.len() + self.command.len() + self.arguments.len()
    }
}

impl FileObservation {
    pub(crate) fn estimated_owned_bytes(&self) -> usize {
        self.path.len() + self.kind.len()
    }
}

impl NetworkObservation {
    pub(crate) fn estimated_owned_bytes(&self) -> usize {
        self.protocol.len()
            + self.local_address.len()
            + self.remote_address.len()
            + self.state.len()
    }
}
