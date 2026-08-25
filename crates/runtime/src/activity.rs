use std::{collections::VecDeque, sync::Mutex};

use serde::Serialize;

use crate::{
    environment::now_ms,
    system::{FileObservation, NetworkObservation, ProcessObservation},
};

const MAX_EXECUTION_EVENTS: usize = 500;
const MAX_EXECUTION_EVENT_BYTES: usize = 1024 * 1024;

pub(crate) struct ActivityLog {
    state: Mutex<ActivityState>,
}

struct ActivityState {
    next_sequence: u64,
    omitted: u64,
    retained_bytes: usize,
    events: VecDeque<RetainedEvent>,
}

struct RetainedEvent {
    estimated_bytes: usize,
    event: ExecutionEvent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ExecutionEvent {
    pub(crate) sequence: u64,
    pub(crate) timestamp_ms: u128,
    #[serde(flatten)]
    pub(crate) activity: ExecutionActivity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ExecutionActivity {
    EnvironmentReady,
    ShellStarted {
        generation: u64,
        columns: u16,
        rows: u16,
    },
    TerminalInput {
        generation: u64,
        input_kind: TerminalInputKind,
        bytes: u64,
    },
    TerminalResized {
        generation: u64,
        columns: u16,
        rows: u16,
    },
    ShellExited {
        generation: u64,
        code: Option<i32>,
    },
    ShellFailed {
        generation: u64,
    },
    ProcessAdded {
        capture_sequence: u64,
        process: ProcessObservation,
    },
    ProcessRemoved {
        capture_sequence: u64,
        process: ProcessObservation,
    },
    ProcessChanged {
        capture_sequence: u64,
        previous: ProcessObservation,
        current: ProcessObservation,
    },
    FileAdded {
        capture_sequence: u64,
        file: FileObservation,
    },
    FileRemoved {
        capture_sequence: u64,
        file: FileObservation,
    },
    FileChanged {
        capture_sequence: u64,
        previous: FileObservation,
        current: FileObservation,
    },
    NetworkAdded {
        capture_sequence: u64,
        network: NetworkObservation,
    },
    NetworkRemoved {
        capture_sequence: u64,
        network: NetworkObservation,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalInputKind {
    Text,
    Binary,
    Interrupt,
}

impl ActivityLog {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                next_sequence: 1,
                omitted: 0,
                retained_bytes: 0,
                events: VecDeque::with_capacity(MAX_EXECUTION_EVENTS),
            }),
        }
    }

    pub(crate) fn record(&self, activity: ExecutionActivity) {
        self.record_batch(std::iter::once(activity), now_ms());
    }

    pub(crate) fn record_batch(
        &self,
        activities: impl IntoIterator<Item = ExecutionActivity>,
        timestamp_ms: u128,
    ) {
        let mut state = self.state.lock().expect("activity log lock poisoned");
        for activity in activities {
            let sequence = state.next_sequence;
            state.next_sequence = state
                .next_sequence
                .checked_add(1)
                .expect("execution event sequence exhausted");
            let event = ExecutionEvent {
                sequence,
                timestamp_ms,
                activity,
            };
            let estimated_bytes = event.estimated_owned_bytes();
            if estimated_bytes > MAX_EXECUTION_EVENT_BYTES {
                state.omitted = state.omitted.saturating_add(1);
                continue;
            }

            while state.events.len() >= MAX_EXECUTION_EVENTS
                || state.retained_bytes + estimated_bytes > MAX_EXECUTION_EVENT_BYTES
            {
                let removed = state
                    .events
                    .pop_front()
                    .expect("retention limit requires an existing event");
                state.retained_bytes -= removed.estimated_bytes;
                state.omitted = state.omitted.saturating_add(1);
            }
            state.retained_bytes += estimated_bytes;
            state.events.push_back(RetainedEvent {
                estimated_bytes,
                event,
            });
        }
    }

    pub(crate) fn snapshot(&self) -> (Vec<ExecutionEvent>, u64) {
        let state = self.state.lock().expect("activity log lock poisoned");
        (
            state
                .events
                .iter()
                .map(|entry| entry.event.clone())
                .collect(),
            state.omitted,
        )
    }
}

impl ExecutionEvent {
    fn estimated_owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.activity.estimated_owned_bytes()
    }
}

impl ExecutionActivity {
    fn estimated_owned_bytes(&self) -> usize {
        match self {
            Self::ProcessAdded { process, .. } | Self::ProcessRemoved { process, .. } => {
                process.estimated_owned_bytes()
            }
            Self::ProcessChanged {
                previous, current, ..
            } => previous.estimated_owned_bytes() + current.estimated_owned_bytes(),
            Self::FileAdded { file, .. } | Self::FileRemoved { file, .. } => {
                file.estimated_owned_bytes()
            }
            Self::FileChanged {
                previous, current, ..
            } => previous.estimated_owned_bytes() + current.estimated_owned_bytes(),
            Self::NetworkAdded { network, .. } | Self::NetworkRemoved { network, .. } => {
                network.estimated_owned_bytes()
            }
            Self::EnvironmentReady
            | Self::ShellStarted { .. }
            | Self::TerminalInput { .. }
            | Self::TerminalResized { .. }
            | Self::ShellExited { .. }
            | Self::ShellFailed { .. } => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_events_in_sequence_order() {
        let log = ActivityLog::new();
        log.record(ExecutionActivity::EnvironmentReady);
        log.record(ExecutionActivity::ShellStarted {
            generation: 1,
            columns: 80,
            rows: 24,
        });

        let (events, omitted) = log.snapshot();
        assert_eq!(omitted, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[0].activity, ExecutionActivity::EnvironmentReady);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(
            events[1].activity,
            ExecutionActivity::ShellStarted {
                generation: 1,
                columns: 80,
                rows: 24,
            }
        );
        assert!(events.iter().all(|event| event.timestamp_ms > 0));
    }

    #[test]
    fn retains_the_newest_five_hundred_whole_events() {
        let log = ActivityLog::new();
        for generation in 1..=MAX_EXECUTION_EVENTS as u64 + 3 {
            log.record(ExecutionActivity::ShellFailed { generation });
        }

        let (events, omitted) = log.snapshot();
        assert_eq!(events.len(), MAX_EXECUTION_EVENTS);
        assert_eq!(omitted, 3);
        assert_eq!(events.first().unwrap().sequence, 4);
        assert_eq!(events.last().unwrap().sequence, 503);
        assert_eq!(
            events.first().unwrap().activity,
            ExecutionActivity::ShellFailed { generation: 4 }
        );
    }

    #[test]
    fn records_a_refresh_batch_atomically_with_one_timestamp() {
        let log = ActivityLog::new();
        log.record_batch(
            [
                ExecutionActivity::ShellFailed { generation: 1 },
                ExecutionActivity::ShellFailed { generation: 2 },
            ],
            42,
        );

        let (events, omitted) = log.snapshot();
        assert_eq!(omitted, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert!(events.iter().all(|event| event.timestamp_ms == 42));
    }

    #[test]
    fn byte_limit_evicts_whole_events_and_counts_oversized_events() {
        let log = ActivityLog::new();
        let large_path = "a".repeat(MAX_EXECUTION_EVENT_BYTES / 2);
        for capture_sequence in 1..=2 {
            log.record(ExecutionActivity::FileAdded {
                capture_sequence,
                file: FileObservation {
                    path: large_path.clone(),
                    size_bytes: 1,
                    modified_unix_seconds: 1,
                    kind: "regular file".into(),
                },
            });
        }

        let (events, omitted) = log.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 2);
        assert_eq!(omitted, 1);

        log.record(ExecutionActivity::FileAdded {
            capture_sequence: 3,
            file: FileObservation {
                path: "b".repeat(MAX_EXECUTION_EVENT_BYTES + 1),
                size_bytes: 1,
                modified_unix_seconds: 1,
                kind: "regular file".into(),
            },
        });
        log.record(ExecutionActivity::EnvironmentReady);

        let (events, omitted) = log.snapshot();
        assert_eq!(omitted, 2);
        assert_eq!(events.last().unwrap().sequence, 4);
        assert_eq!(
            events.last().unwrap().activity,
            ExecutionActivity::EnvironmentReady
        );
    }
}
