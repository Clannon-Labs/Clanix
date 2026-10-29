use uuid::Uuid;

/// This will be later used to send the output of the execution which will stream the 
/// bytes as clanix receives them
#[derive(Debug)]
pub enum ExecutionEvent {
    Started {
        id: Uuid,
    },
    Output {
        id: Uuid,
        bytes: Vec<u8>,
    },
}

