use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    error::{RuntimeError, RuntimeErrorKind},
    observation::{self, ObservationSnapshot},
    podman,
    terminal::{self, TerminalReservation},
};

const MAX_TRANSCRIPT_ENTRIES: usize = 500;

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<StateInner>,
}

struct StateInner {
    environments: Mutex<HashMap<String, Arc<Environment>>>,
    next_id: AtomicU64,
    container_prefix: String,
    image: String,
}

pub(crate) struct Environment {
    id: String,
    container_name: String,
    transcript: Mutex<Vec<TranscriptEntry>>,
    terminal_active: AtomicBool,
}

#[derive(Clone, Serialize)]
pub(crate) struct TranscriptEntry {
    timestamp_ms: u128,
    direction: &'static str,
    data: String,
}

impl Runtime {
    pub fn new(image: String) -> Self {
        let started_at = now_ms();
        Self {
            inner: Arc::new(StateInner {
                environments: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                container_prefix: format!("clannon-{}-{started_at}", std::process::id()),
                image,
            }),
        }
    }

    pub async fn verify_rootless() -> Result<(), String> {
        podman::verify_rootless().await
    }

    pub async fn create(&self) -> Result<String, RuntimeError> {
        let sequence = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("env-{sequence:08x}");
        let container_name = format!("{}-{sequence:08x}", self.inner.container_prefix);

        podman::create_container(&container_name, &self.inner.image).await?;

        let environment = Arc::new(Environment {
            id: id.clone(),
            container_name,
            transcript: Mutex::new(Vec::new()),
            terminal_active: AtomicBool::new(false),
        });
        self.inner
            .environments
            .lock()
            .await
            .insert(id.clone(), environment);

        Ok(id)
    }

    async fn find(&self, id: &str) -> Result<Arc<Environment>, RuntimeError> {
        self.inner
            .environments
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::new(RuntimeErrorKind::NotFound, "environment not found"))
    }

    pub async fn destroy(&self, id: String) -> Result<(), RuntimeError> {
        let environment = self
            .inner
            .environments
            .lock()
            .await
            .remove(&id)
            .ok_or_else(|| {
                RuntimeError::new(RuntimeErrorKind::NotFound, "environment not found")
            })?;

        if let Err(error) = podman::remove_container(environment.container_name()).await {
            // Keep ownership when Podman fails so the user can retry destruction
            // instead of leaving an unreachable container behind.
            self.inner.environments.lock().await.insert(id, environment);
            return Err(error);
        }

        Ok(())
    }

    pub async fn observe(&self, id: &str) -> Result<ObservationSnapshot, RuntimeError> {
        Ok(observation::collect(self.find(id).await?).await)
    }

    pub async fn reserve_terminal(&self, id: &str) -> Result<TerminalReservation, RuntimeError> {
        terminal::reserve(self.find(id).await?)
    }

    pub async fn cleanup(&self) {
        let environments: Vec<_> = self
            .inner
            .environments
            .lock()
            .await
            .drain()
            .map(|(_, environment)| environment)
            .collect();
        for environment in environments {
            let _ = podman::remove_container(environment.container_name()).await;
        }
    }
}

impl Environment {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn container_name(&self) -> &str {
        &self.container_name
    }

    pub(crate) fn try_open_terminal(&self) -> bool {
        !self.terminal_active.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn close_terminal(&self) {
        self.terminal_active.store(false, Ordering::Release);
    }

    pub(crate) async fn record_transcript(&self, direction: &'static str, data: String) {
        let mut transcript = self.transcript.lock().await;
        transcript.push(TranscriptEntry {
            timestamp_ms: now_ms(),
            direction,
            data,
        });
        if transcript.len() > MAX_TRANSCRIPT_ENTRIES {
            let excess = transcript.len() - MAX_TRANSCRIPT_ENTRIES;
            transcript.drain(..excess);
        }
    }

    pub(crate) async fn transcript_snapshot(&self) -> Vec<TranscriptEntry> {
        self.transcript.lock().await.clone()
    }
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_transcript_without_losing_newest_entries() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let environment = Environment {
                id: "test".into(),
                container_name: "none".into(),
                transcript: Mutex::new(Vec::new()),
                terminal_active: AtomicBool::new(false),
            };
            for index in 0..=MAX_TRANSCRIPT_ENTRIES {
                environment
                    .record_transcript("output", index.to_string())
                    .await;
            }
            let transcript = environment.transcript.lock().await;
            assert_eq!(transcript.len(), MAX_TRANSCRIPT_ENTRIES);
            assert_eq!(transcript.first().unwrap().data, "1");
            assert_eq!(
                transcript.last().unwrap().data,
                MAX_TRANSCRIPT_ENTRIES.to_string()
            );
        });
    }
}
