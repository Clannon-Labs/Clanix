use std::{collections::VecDeque, sync::Mutex};

use serde::Serialize;

use crate::environment::now_ms;

const MAX_EXECUTION_EVENTS: usize = 500;

pub(crate) struct ActivityLog {
    state: Mutex<ActivityState>,
}

struct ActivityState {
    next_sequence: u64,
    omitted: u64,
    events: VecDeque<ExecutionEvent>,
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
                events: VecDeque::with_capacity(MAX_EXECUTION_EVENTS),
            }),
        }
    }

    pub(crate) fn record(&self, activity: ExecutionActivity) {
        let mut state = self.state.lock().expect("activity log lock poisoned");
        let sequence = state.next_sequence;
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .expect("execution event sequence exhausted");

        if state.events.len() == MAX_EXECUTION_EVENTS {
            state.events.pop_front();
            state.omitted = state.omitted.saturating_add(1);
        }
        state.events.push_back(ExecutionEvent {
            sequence,
            timestamp_ms: now_ms(),
            activity,
        });
    }

    pub(crate) fn snapshot(&self) -> (Vec<ExecutionEvent>, u64) {
        let state = self.state.lock().expect("activity log lock poisoned");
        (state.events.iter().cloned().collect(), state.omitted)
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
}
