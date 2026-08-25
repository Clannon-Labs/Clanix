use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    future::Future,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{
    sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, oneshot},
    task::JoinHandle,
};

use crate::{
    error::{RuntimeError, RuntimeErrorKind},
    observation::{self, ObservationSnapshot},
    podman,
    terminal::{self, TerminalReservation},
};

const MAX_TRANSCRIPT_ENTRIES: usize = 500;
const MAX_TRANSCRIPT_BYTES: usize = 1024 * 1024;
const MAX_ENVIRONMENTS: usize = 4;

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<StateInner>,
}

struct StateInner {
    environments: Mutex<HashMap<String, Arc<Environment>>>,
    environment_slots: Arc<Semaphore>,
    issued_environment_ids: StdMutex<HashSet<String>>,
    lifecycles: StdMutex<LifecycleState>,
    cleanup_started: AtomicBool,
    cleanup_finished: AtomicBool,
    cleanup_notify: Notify,
    next_container: AtomicU64,
    container_prefix: String,
    image: String,
}

struct LifecycleState {
    accepting: bool,
    tasks: Vec<JoinHandle<()>>,
}

struct PendingCreate {
    id: String,
    container_name: String,
    image: String,
    slot: OwnedSemaphorePermit,
}

struct CreatedEnvironment {
    id: String,
    acknowledgement: oneshot::Sender<()>,
}

pub(crate) struct Environment {
    id: String,
    container_name: String,
    transcript: Arc<Transcript>,
    terminal_active: AtomicBool,
    terminal: terminal::TerminalHub,
    slot: StdMutex<Option<OwnedSemaphorePermit>>,
}

pub(crate) struct Transcript {
    entries: Mutex<Vec<TranscriptEntry>>,
}

#[derive(Clone, Serialize)]
pub(crate) struct TranscriptEntry {
    pub(crate) timestamp_ms: u128,
    pub(crate) direction: &'static str,
    pub(crate) data: String,
}

impl Runtime {
    pub fn new(image: String) -> Self {
        let started_at = now_ms();
        Self {
            inner: Arc::new(StateInner {
                environments: Mutex::new(HashMap::new()),
                environment_slots: Arc::new(Semaphore::new(MAX_ENVIRONMENTS)),
                issued_environment_ids: StdMutex::new(HashSet::new()),
                lifecycles: StdMutex::new(LifecycleState {
                    accepting: true,
                    tasks: Vec::new(),
                }),
                cleanup_started: AtomicBool::new(false),
                cleanup_finished: AtomicBool::new(false),
                cleanup_notify: Notify::new(),
                next_container: AtomicU64::new(1),
                container_prefix: format!("clannon-{}-{started_at}", std::process::id()),
                image,
            }),
        }
    }

    pub async fn verify_rootless() -> Result<(), String> {
        podman::verify_rootless().await
    }

    pub async fn create(&self) -> Result<String, RuntimeError> {
        self.create_with(
            |container, image| async move { podman::create_container(&container, &image).await },
            |container| async move { podman::remove_container(&container).await },
        )
        .await
    }

    async fn create_with<Create, CreateFuture, Remove, RemoveFuture>(
        &self,
        create_container: Create,
        remove_container: Remove,
    ) -> Result<String, RuntimeError>
    where
        Create: FnOnce(String, String) -> CreateFuture + Send + 'static,
        CreateFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
        Remove: FnOnce(String) -> RemoveFuture + Send + 'static,
        RemoveFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
    {
        let slot = self.acquire_environment_slot()?;
        let id = self.allocate_environment_id()?;
        let sequence = self.inner.next_container.fetch_add(1, Ordering::Relaxed);
        let container_name = format!("{}-{sequence:08x}", self.inner.container_prefix);
        let pending = PendingCreate {
            id,
            container_name,
            image: self.inner.image.clone(),
            slot,
        };
        let inner = self.inner.clone();
        let (result_sender, result_receiver) = oneshot::channel();

        {
            let mut lifecycles = self
                .inner
                .lifecycles
                .lock()
                .expect("lifecycle task lock poisoned");
            if !lifecycles.accepting {
                release_environment_id(&self.inner, &pending.id);
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    "environment creation is unavailable during cleanup",
                ));
            }
            lifecycles.tasks.retain(|task| !task.is_finished());
            lifecycles.tasks.push(tokio::spawn(async move {
                finish_create(
                    inner,
                    pending,
                    result_sender,
                    create_container,
                    remove_container,
                )
                .await;
            }));
        }

        match result_receiver.await {
            Ok(Ok(created)) => {
                let CreatedEnvironment {
                    id,
                    acknowledgement,
                } = created;
                let _ = acknowledgement.send(());
                Ok(id)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(RuntimeError::new(
                RuntimeErrorKind::Internal,
                "environment creation task ended unexpectedly",
            )),
        }
    }

    fn acquire_environment_slot(&self) -> Result<OwnedSemaphorePermit, RuntimeError> {
        self.inner
            .environment_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    format!("at most {MAX_ENVIRONMENTS} environments may exist at once"),
                )
            })
    }

    fn allocate_environment_id(&self) -> Result<String, RuntimeError> {
        loop {
            let id = generate_environment_id()?;
            if self
                .inner
                .issued_environment_ids
                .lock()
                .expect("environment ID lock poisoned")
                .insert(id.clone())
            {
                return Ok(id);
            }
        }
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
        self.destroy_with(id, |container| async move {
            podman::remove_container(&container).await
        })
        .await
    }

    async fn destroy_with<Remove, RemoveFuture>(
        &self,
        id: String,
        remove_container: Remove,
    ) -> Result<(), RuntimeError>
    where
        Remove: FnOnce(String) -> RemoveFuture + Send + 'static,
        RemoveFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
    {
        let inner = self.inner.clone();
        let (result_sender, result_receiver) = oneshot::channel();
        {
            let mut lifecycles = self
                .inner
                .lifecycles
                .lock()
                .expect("lifecycle task lock poisoned");
            if !lifecycles.accepting {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    "environment destruction is unavailable during cleanup",
                ));
            }
            lifecycles.tasks.retain(|task| !task.is_finished());
            lifecycles.tasks.push(tokio::spawn(async move {
                finish_destroy(inner, id, result_sender, remove_container).await;
            }));
        }

        result_receiver.await.unwrap_or_else(|_| {
            Err(RuntimeError::new(
                RuntimeErrorKind::Internal,
                "environment destruction task ended unexpectedly",
            ))
        })
    }

    pub async fn observe(&self, id: &str) -> Result<ObservationSnapshot, RuntimeError> {
        Ok(observation::collect(self.find(id).await?).await)
    }

    pub async fn reserve_terminal(&self, id: &str) -> Result<TerminalReservation, RuntimeError> {
        terminal::reserve(self.find(id).await?)
    }

    pub async fn cleanup(&self) {
        self.cleanup_with(|container| async move { podman::remove_container(&container).await })
            .await;
    }

    async fn cleanup_with<Remove, RemoveFuture>(&self, remove_container: Remove)
    where
        Remove: Fn(String) -> RemoveFuture + Clone + Send + Sync + 'static,
        RemoveFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
    {
        if !self.inner.cleanup_started.swap(true, Ordering::AcqRel) {
            let lifecycle_tasks = {
                let mut lifecycles = self
                    .inner
                    .lifecycles
                    .lock()
                    .expect("lifecycle task lock poisoned");
                lifecycles.accepting = false;
                std::mem::take(&mut lifecycles.tasks)
            };
            let inner = self.inner.clone();
            tokio::spawn(async move {
                // Keep the coordinator alive even if cleanup implementation code
                // panics, so later cleanup callers never wait forever.
                let cleanup = tokio::spawn(finish_cleanup(
                    inner.clone(),
                    lifecycle_tasks,
                    remove_container,
                ));
                let _ = cleanup.await;
                inner.cleanup_finished.store(true, Ordering::Release);
                inner.cleanup_notify.notify_waiters();
            });
        }

        loop {
            let finished = self.inner.cleanup_notify.notified();
            if self.inner.cleanup_finished.load(Ordering::Acquire) {
                return;
            }
            finished.await;
        }
    }
}

async fn finish_cleanup<Remove, RemoveFuture>(
    inner: Arc<StateInner>,
    lifecycle_tasks: Vec<JoinHandle<()>>,
    remove_container: Remove,
) where
    Remove: Fn(String) -> RemoveFuture + Clone + Send + Sync + 'static,
    RemoveFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
{
    for task in lifecycle_tasks {
        let _ = task.await;
    }

    let environments: Vec<_> = inner
        .environments
        .lock()
        .await
        .drain()
        .map(|(_, environment)| environment)
        .collect();
    let removals: Vec<_> = environments
        .into_iter()
        .map(|environment| {
            let inner = inner.clone();
            let remove_container = remove_container.clone();
            tokio::spawn(async move {
                let removed = remove_container(environment.container_name().to_owned())
                    .await
                    .is_ok();
                if removed {
                    environment
                        .terminal()
                        .finish_after_container_removed()
                        .await;
                } else {
                    environment.terminal().stop_and_reap().await;
                }
                release_environment_id(&inner, environment.id());
                environment.release_slot();
            })
        })
        .collect();
    for removal in removals {
        let _ = removal.await;
    }
}

async fn finish_create<Create, CreateFuture, Remove, RemoveFuture>(
    inner: Arc<StateInner>,
    pending: PendingCreate,
    result_sender: oneshot::Sender<Result<CreatedEnvironment, RuntimeError>>,
    create_container: Create,
    remove_container: Remove,
) where
    Create: FnOnce(String, String) -> CreateFuture,
    CreateFuture: Future<Output = Result<(), RuntimeError>>,
    Remove: FnOnce(String) -> RemoveFuture,
    RemoveFuture: Future<Output = Result<(), RuntimeError>>,
{
    let PendingCreate {
        id,
        container_name,
        image,
        slot,
    } = pending;
    if let Err(error) = create_container(container_name.clone(), image).await {
        let environment = new_environment(id, container_name, slot);
        clean_unpublished_environment(inner, environment, remove_container).await;
        let _ = result_sender.send(Err(error));
        return;
    }

    let environment = new_environment(id.clone(), container_name, slot);
    if result_sender.is_closed() {
        clean_unpublished_environment(inner, environment, remove_container).await;
        return;
    }

    let inserted = {
        let mut environments = inner.environments.lock().await;
        match environments.entry(id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(environment.clone());
                true
            }
            Entry::Occupied(_) => false,
        }
    };
    if !inserted {
        clean_unpublished_environment(inner, environment, remove_container).await;
        let _ = result_sender.send(Err(RuntimeError::new(
            RuntimeErrorKind::Internal,
            "could not publish the reserved environment ID",
        )));
        return;
    }

    let (acknowledgement, acknowledged) = oneshot::channel();
    if result_sender
        .send(Ok(CreatedEnvironment {
            id: id.clone(),
            acknowledgement,
        }))
        .is_err()
    {
        if let Some(environment) = take_environment(&inner, &id, &environment).await {
            clean_unpublished_environment(inner, environment, remove_container).await;
        }
        return;
    }

    if acknowledged.await.is_err()
        && let Some(environment) = take_environment(&inner, &id, &environment).await
    {
        clean_unpublished_environment(inner, environment, remove_container).await;
    }
}

async fn finish_destroy<Remove, RemoveFuture>(
    inner: Arc<StateInner>,
    id: String,
    result_sender: oneshot::Sender<Result<(), RuntimeError>>,
    remove_container: Remove,
) where
    Remove: FnOnce(String) -> RemoveFuture,
    RemoveFuture: Future<Output = Result<(), RuntimeError>>,
{
    let Some(environment) = inner.environments.lock().await.remove(&id) else {
        let _ = result_sender.send(Err(RuntimeError::new(
            RuntimeErrorKind::NotFound,
            "environment not found",
        )));
        return;
    };

    if let Err(error) = remove_container(environment.container_name().to_owned()).await {
        // Keep ownership when Podman fails so destruction can be retried instead
        // of leaving an unreachable container behind.
        inner.environments.lock().await.insert(id, environment);
        let _ = result_sender.send(Err(error));
        return;
    }

    environment
        .terminal()
        .finish_after_container_removed()
        .await;
    release_environment_id(&inner, environment.id());
    environment.release_slot();
    let _ = result_sender.send(Ok(()));
}

fn new_environment(
    id: String,
    container_name: String,
    slot: OwnedSemaphorePermit,
) -> Arc<Environment> {
    let transcript = Arc::new(Transcript::new());
    Arc::new(Environment {
        id,
        container_name,
        transcript: transcript.clone(),
        terminal_active: AtomicBool::new(false),
        terminal: terminal::TerminalHub::new(transcript),
        slot: StdMutex::new(Some(slot)),
    })
}

async fn take_environment(
    inner: &StateInner,
    id: &str,
    expected: &Arc<Environment>,
) -> Option<Arc<Environment>> {
    let mut environments = inner.environments.lock().await;
    if environments
        .get(id)
        .is_some_and(|environment| Arc::ptr_eq(environment, expected))
    {
        environments.remove(id)
    } else {
        None
    }
}

async fn clean_unpublished_environment<Remove, RemoveFuture>(
    inner: Arc<StateInner>,
    environment: Arc<Environment>,
    remove_container: Remove,
) where
    Remove: FnOnce(String) -> RemoveFuture,
    RemoveFuture: Future<Output = Result<(), RuntimeError>>,
{
    if remove_container(environment.container_name().to_owned())
        .await
        .is_ok()
    {
        environment
            .terminal()
            .finish_after_container_removed()
            .await;
        release_environment_id(&inner, environment.id());
        environment.release_slot();
        return;
    }

    let mut environments = inner.environments.lock().await;
    if let Entry::Vacant(entry) = environments.entry(environment.id().to_owned()) {
        entry.insert(environment);
    }
}

fn release_environment_id(inner: &StateInner, id: &str) {
    inner
        .issued_environment_ids
        .lock()
        .expect("environment ID lock poisoned")
        .remove(id);
}

impl Environment {
    #[cfg(test)]
    pub(crate) fn for_test() -> Arc<Self> {
        let transcript = Arc::new(Transcript::new());
        Arc::new(Self {
            id: "test".into(),
            container_name: "none".into(),
            transcript: transcript.clone(),
            terminal_active: AtomicBool::new(false),
            terminal: terminal::TerminalHub::new(transcript),
            slot: StdMutex::new(None),
        })
    }

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

    fn release_slot(&self) {
        drop(
            self.slot
                .lock()
                .expect("environment slot lock poisoned")
                .take(),
        );
    }

    pub(crate) async fn record_transcript(&self, direction: &'static str, data: String) {
        self.transcript.record(direction, data).await;
    }

    pub(crate) fn terminal(&self) -> &terminal::TerminalHub {
        &self.terminal
    }

    pub(crate) async fn transcript_snapshot(&self) -> Vec<TranscriptEntry> {
        self.transcript.snapshot().await
    }
}

impl Transcript {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    pub(crate) async fn record(&self, direction: &'static str, data: String) {
        if data.len() > MAX_TRANSCRIPT_BYTES {
            return;
        }
        let mut entries = self.entries.lock().await;
        entries.push(TranscriptEntry {
            timestamp_ms: now_ms(),
            direction,
            data,
        });

        let mut retained_bytes = entries.iter().map(|entry| entry.data.len()).sum::<usize>();
        while entries.len() > MAX_TRANSCRIPT_ENTRIES || retained_bytes > MAX_TRANSCRIPT_BYTES {
            retained_bytes -= entries.remove(0).data.len();
        }
    }

    pub(crate) async fn snapshot(&self) -> Vec<TranscriptEntry> {
        self.entries.lock().await.clone()
    }
}

fn generate_environment_id() -> Result<String, RuntimeError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| RuntimeError::internal("could not generate environment ID", error))?;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(4 + bytes.len() * 2);
    id.push_str("env-");
    for byte in bytes {
        id.push(HEX[(byte >> 4) as usize] as char);
        id.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(id)
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
    fn environment_ids_are_opaque_128_bit_values() {
        let first = generate_environment_id().unwrap();
        let second = generate_environment_id().unwrap();

        for id in [&first, &second] {
            assert_eq!(id.len(), 36);
            assert!(id.starts_with("env-"));
            assert!(id[4..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn owned_slots_cap_creating_and_live_environments() {
        let runtime = Runtime::new("unused".into());
        let mut slots = Vec::new();
        for _ in 0..MAX_ENVIRONMENTS {
            slots.push(runtime.acquire_environment_slot().unwrap());
        }
        let error = runtime.acquire_environment_slot().unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::Conflict);

        drop(slots.pop());
        let released_after_create_failure = runtime.acquire_environment_slot().unwrap();
        let transcript = Arc::new(Transcript::new());
        let environment = Environment {
            id: "test".into(),
            container_name: "none".into(),
            transcript: transcript.clone(),
            terminal_active: AtomicBool::new(false),
            terminal: terminal::TerminalHub::new(transcript),
            slot: StdMutex::new(Some(released_after_create_failure)),
        };
        environment.release_slot();
        let released_after_environment_end = runtime.acquire_environment_slot().unwrap();
        drop(released_after_environment_end);
        drop(slots);
    }

    #[test]
    fn caps_transcript_without_losing_newest_entries() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let transcript = Transcript::new();
            for index in 0..=MAX_TRANSCRIPT_ENTRIES {
                transcript.record("output", index.to_string()).await;
            }
            let entries = transcript.entries.lock().await;
            assert_eq!(entries.len(), MAX_TRANSCRIPT_ENTRIES);
            assert_eq!(entries.first().unwrap().data, "1");
            assert_eq!(
                entries.last().unwrap().data,
                MAX_TRANSCRIPT_ENTRIES.to_string()
            );
        });
    }

    #[tokio::test]
    async fn caps_transcript_by_utf8_bytes_and_keeps_newest_entries() {
        let transcript = Transcript::new();
        let half_plus_one = MAX_TRANSCRIPT_BYTES / 2 + 1;
        transcript.record("output", "a".repeat(half_plus_one)).await;
        transcript.record("output", "b".repeat(half_plus_one)).await;

        let entries = transcript.snapshot().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data, "b".repeat(half_plus_one));
        assert!(
            entries.iter().map(|entry| entry.data.len()).sum::<usize>() <= MAX_TRANSCRIPT_BYTES
        );
    }

    #[tokio::test]
    async fn oversized_transcript_entry_is_dropped_without_rewriting_evidence() {
        let transcript = Transcript::new();
        let data = format!("{}a", "é".repeat(MAX_TRANSCRIPT_BYTES / 2 + 1));
        transcript.record("output", data).await;

        let entries = transcript.snapshot().await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn canceled_create_is_cleaned_before_its_slot_is_released() {
        let runtime = Runtime::new("unused".into());
        let request_runtime = runtime.clone();
        let (started_sender, started_receiver) = oneshot::channel();
        let (finish_sender, finish_receiver) = oneshot::channel();
        let (removed_sender, removed_receiver) = oneshot::channel();
        let request = tokio::spawn(async move {
            request_runtime
                .create_with(
                    move |_, _| async move {
                        let _ = started_sender.send(());
                        let _ = finish_receiver.await;
                        Ok(())
                    },
                    move |_| async move {
                        let _ = removed_sender.send(());
                        Ok(())
                    },
                )
                .await
        });

        started_receiver.await.unwrap();
        request.abort();
        let _ = request.await;
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);
        finish_sender.send(()).unwrap();
        removed_receiver.await.unwrap();
        wait_until(|| runtime.inner.environment_slots.available_permits() == MAX_ENVIRONMENTS)
            .await;
        assert!(runtime.inner.environments.lock().await.is_empty());
        assert!(
            runtime
                .inner
                .issued_environment_ids
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn canceled_create_retains_ownership_when_cleanup_fails() {
        let runtime = Runtime::new("unused".into());
        let request_runtime = runtime.clone();
        let (started_sender, started_receiver) = oneshot::channel();
        let (finish_sender, finish_receiver) = oneshot::channel();
        let (removed_sender, removed_receiver) = oneshot::channel();
        let request = tokio::spawn(async move {
            request_runtime
                .create_with(
                    move |_, _| async move {
                        let _ = started_sender.send(());
                        let _ = finish_receiver.await;
                        Ok(())
                    },
                    move |container| async move {
                        let _ = removed_sender.send(container);
                        Err(RuntimeError::new(
                            RuntimeErrorKind::BadGateway,
                            "test cleanup failed",
                        ))
                    },
                )
                .await
        });

        started_receiver.await.unwrap();
        request.abort();
        let _ = request.await;
        finish_sender.send(()).unwrap();
        let container = removed_receiver.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !runtime.inner.environments.lock().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed cleanup must retain the environment");
        let environments = runtime.inner.environments.lock().await;
        assert_eq!(environments.len(), 1);
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);
        assert_eq!(
            runtime.inner.issued_environment_ids.lock().unwrap().len(),
            1
        );
        assert_eq!(
            environments.values().next().unwrap().container_name(),
            container
        );
    }

    #[tokio::test]
    async fn failed_create_cleans_unknown_outcome_before_releasing_ownership() {
        let runtime = Runtime::new("unused".into());
        let (removed_sender, removed_receiver) = oneshot::channel();
        let error = runtime
            .create_with(
                |_, _| async {
                    Err(RuntimeError::new(
                        RuntimeErrorKind::BadGateway,
                        "test create failed",
                    ))
                },
                move |container| async move {
                    let _ = removed_sender.send(container);
                    Ok(())
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::BadGateway);
        assert!(removed_receiver.await.unwrap().starts_with("clannon-"));
        assert_eq!(
            runtime.inner.environment_slots.available_permits(),
            MAX_ENVIRONMENTS
        );
        assert!(
            runtime
                .inner
                .issued_environment_ids
                .lock()
                .unwrap()
                .is_empty()
        );
        assert!(runtime.inner.environments.lock().await.is_empty());
    }

    #[tokio::test]
    async fn failed_create_retains_ownership_when_unknown_outcome_cleanup_fails() {
        let runtime = Runtime::new("unused".into());
        let error = runtime
            .create_with(
                |_, _| async {
                    Err(RuntimeError::new(
                        RuntimeErrorKind::BadGateway,
                        "test create outcome unknown",
                    ))
                },
                |_| async {
                    Err(RuntimeError::new(
                        RuntimeErrorKind::BadGateway,
                        "test cleanup failed",
                    ))
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::BadGateway);
        assert_eq!(runtime.inner.environments.lock().await.len(), 1);
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);
        assert_eq!(
            runtime.inner.issued_environment_ids.lock().unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn unacknowledged_create_delivery_is_cleaned() {
        let runtime = Runtime::new("unused".into());
        let id = runtime.allocate_environment_id().unwrap();
        let pending = PendingCreate {
            id: id.clone(),
            container_name: "test-unacknowledged-create".into(),
            image: "unused".into(),
            slot: runtime.acquire_environment_slot().unwrap(),
        };
        let (result_sender, result_receiver) = oneshot::channel();
        let (removed_sender, removed_receiver) = oneshot::channel();
        let create_task = tokio::spawn(finish_create(
            runtime.inner.clone(),
            pending,
            result_sender,
            |_, _| async { Ok(()) },
            move |container| async move {
                let _ = removed_sender.send(container);
                Ok(())
            },
        ));

        let delivered = result_receiver.await.unwrap().unwrap();
        assert_eq!(delivered.id, id);
        assert_eq!(runtime.inner.environments.lock().await.len(), 1);
        drop(delivered);

        assert_eq!(
            removed_receiver.await.unwrap(),
            "test-unacknowledged-create"
        );
        create_task.await.unwrap();
        assert!(runtime.inner.environments.lock().await.is_empty());
        assert_eq!(
            runtime.inner.environment_slots.available_permits(),
            MAX_ENVIRONMENTS
        );
        assert!(
            runtime
                .inner
                .issued_environment_ids
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn canceled_destroy_completes_before_releasing_ownership() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "destroy-success").await;
        let request_runtime = runtime.clone();
        let (started_sender, started_receiver) = oneshot::channel();
        let (finish_sender, finish_receiver) = oneshot::channel();
        let request = tokio::spawn(async move {
            request_runtime
                .destroy_with("destroy-success".into(), move |_| async move {
                    let _ = started_sender.send(());
                    let _ = finish_receiver.await;
                    Ok(())
                })
                .await
        });

        started_receiver.await.unwrap();
        request.abort();
        let _ = request.await;
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);
        finish_sender.send(()).unwrap();
        wait_until(|| runtime.inner.environment_slots.available_permits() == MAX_ENVIRONMENTS)
            .await;
        assert!(runtime.inner.environments.lock().await.is_empty());
        assert!(
            runtime
                .inner
                .issued_environment_ids
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn canceled_destroy_retains_ownership_when_removal_fails() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "destroy-failure").await;
        let request_runtime = runtime.clone();
        let (started_sender, started_receiver) = oneshot::channel();
        let (finish_sender, finish_receiver) = oneshot::channel();
        let request = tokio::spawn(async move {
            request_runtime
                .destroy_with("destroy-failure".into(), move |_| async move {
                    let _ = started_sender.send(());
                    let _ = finish_receiver.await;
                    Err(RuntimeError::new(
                        RuntimeErrorKind::BadGateway,
                        "test destroy failed",
                    ))
                })
                .await
        });

        started_receiver.await.unwrap();
        request.abort();
        let _ = request.await;
        finish_sender.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if runtime
                    .inner
                    .environments
                    .lock()
                    .await
                    .contains_key("destroy-failure")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed destroy must restore runtime ownership");
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);
        assert!(
            runtime
                .inner
                .issued_environment_ids
                .lock()
                .unwrap()
                .contains("destroy-failure")
        );
    }

    #[tokio::test]
    async fn canceled_cleanup_remains_owned_and_observable() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "cleanup-cancellation").await;
        let removal_started = Arc::new(Notify::new());
        let finish_removal = Arc::new(Notify::new());
        let request_runtime = runtime.clone();
        let request_started = removal_started.clone();
        let request_finish = finish_removal.clone();
        let cleanup = tokio::spawn(async move {
            request_runtime
                .cleanup_with(move |_| {
                    let removal_started = request_started.clone();
                    let finish_removal = request_finish.clone();
                    async move {
                        removal_started.notify_one();
                        finish_removal.notified().await;
                        Ok(())
                    }
                })
                .await;
        });

        removal_started.notified().await;
        cleanup.abort();
        let _ = cleanup.await;
        assert!(runtime.inner.environments.lock().await.is_empty());
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);

        let waiting_runtime = runtime.clone();
        let waiting_cleanup = tokio::spawn(async move {
            waiting_runtime
                .cleanup_with(|_| async { panic!("cleanup operation must only start once") })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!waiting_cleanup.is_finished());

        finish_removal.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting_cleanup)
            .await
            .expect("later cleanup caller must observe completion")
            .unwrap();
        assert!(runtime.inner.cleanup_finished.load(Ordering::Acquire));
        assert_eq!(
            runtime.inner.environment_slots.available_permits(),
            MAX_ENVIRONMENTS
        );
        assert!(
            runtime
                .inner
                .issued_environment_ids
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    async fn insert_test_environment(runtime: &Runtime, id: &str) {
        let inserted = runtime
            .inner
            .issued_environment_ids
            .lock()
            .unwrap()
            .insert(id.to_owned());
        assert!(inserted);
        let environment = new_environment(
            id.to_owned(),
            format!("test-container-{id}"),
            runtime.acquire_environment_slot().unwrap(),
        );
        assert!(
            runtime
                .inner
                .environments
                .lock()
                .await
                .insert(id.to_owned(), environment)
                .is_none()
        );
    }

    async fn wait_until(predicate: impl Fn() -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("condition must become true");
    }
}
