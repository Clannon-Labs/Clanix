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
    activity::{ActivityLog, ExecutionActivity},
    error::{RuntimeError, RuntimeErrorKind},
    observation::{self, ObservationSnapshot},
    podman,
    snapshot::{
        MAX_RETAINED_SNAPSHOT_BYTES, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOTS, Snapshot, SnapshotSummary,
    },
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
    snapshots: Mutex<HashMap<String, Arc<Snapshot>>>,
    snapshot_slots: Arc<Semaphore>,
    snapshot_bytes: Arc<Semaphore>,
    snapshot_capture: Arc<Semaphore>,
    issued_snapshot_ids: StdMutex<HashSet<String>>,
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

struct PendingSnapshot {
    id: String,
    source_environment_id: String,
    source_container_name: String,
    slot: OwnedSemaphorePermit,
    byte_reservation: OwnedSemaphorePermit,
    capture_gate: OwnedSemaphorePermit,
}

struct CreatedSnapshot {
    summary: SnapshotSummary,
    acknowledgement: oneshot::Sender<()>,
}

pub(crate) struct Environment {
    id: String,
    container_name: String,
    transcript: Arc<Transcript>,
    activity: Arc<ActivityLog>,
    observation: Mutex<observation::ObservationState>,
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
                snapshots: Mutex::new(HashMap::new()),
                snapshot_slots: Arc::new(Semaphore::new(MAX_SNAPSHOTS)),
                snapshot_bytes: Arc::new(Semaphore::new(MAX_RETAINED_SNAPSHOT_BYTES)),
                snapshot_capture: Arc::new(Semaphore::new(1)),
                issued_snapshot_ids: StdMutex::new(HashSet::new()),
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

    pub async fn save_snapshot(
        &self,
        environment_id: &str,
    ) -> Result<SnapshotSummary, RuntimeError> {
        self.save_snapshot_with(environment_id, |container, archive_limit| async move {
            podman::capture_workspace(&container, archive_limit).await
        })
        .await
    }

    async fn save_snapshot_with<Capture, CaptureFuture>(
        &self,
        environment_id: &str,
        capture_workspace: Capture,
    ) -> Result<SnapshotSummary, RuntimeError>
    where
        Capture: FnOnce(String, usize) -> CaptureFuture + Send + 'static,
        CaptureFuture: Future<Output = Result<Vec<u8>, RuntimeError>> + Send + 'static,
    {
        let source = self.find(environment_id).await?;
        let slot = self
            .inner
            .snapshot_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    format!("at most {MAX_SNAPSHOTS} snapshots may exist at once"),
                )
            })?;
        let capture_gate = self
            .inner
            .snapshot_capture
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                RuntimeError::new(
                    RuntimeErrorKind::Internal,
                    "snapshot capture gate closed unexpectedly",
                )
            })?;
        let archive_limit = self
            .inner
            .snapshot_bytes
            .available_permits()
            .min(MAX_SNAPSHOT_BYTES);
        if archive_limit == 0 {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "snapshot memory limit is currently exhausted",
            ));
        }
        let byte_reservation = self
            .inner
            .snapshot_bytes
            .clone()
            .try_acquire_many_owned(archive_limit as u32)
            .map_err(|_| {
                RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    "snapshot memory limit is currently exhausted",
                )
            })?;
        let id = self.allocate_snapshot_id()?;
        let pending = PendingSnapshot {
            id,
            source_environment_id: environment_id.to_owned(),
            source_container_name: source.container_name().to_owned(),
            slot,
            byte_reservation,
            capture_gate,
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
                release_snapshot_id(&self.inner, &pending.id);
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    "snapshot creation is unavailable during cleanup",
                ));
            }
            lifecycles.tasks.retain(|task| !task.is_finished());
            lifecycles.tasks.push(tokio::spawn(async move {
                finish_save_snapshot(inner, pending, result_sender, capture_workspace).await;
            }));
        }

        match result_receiver.await {
            Ok(Ok(created)) => {
                let CreatedSnapshot {
                    summary,
                    acknowledgement,
                } = created;
                let _ = acknowledgement.send(());
                Ok(summary)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(RuntimeError::new(
                RuntimeErrorKind::Internal,
                "snapshot creation task ended unexpectedly",
            )),
        }
    }

    pub async fn list_snapshots(&self) -> Vec<SnapshotSummary> {
        let mut snapshots: Vec<_> = self
            .inner
            .snapshots
            .lock()
            .await
            .values()
            .map(|snapshot| snapshot.summary())
            .collect();
        snapshots.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        snapshots
    }

    pub async fn delete_snapshot(&self, snapshot_id: String) -> Result<(), RuntimeError> {
        if !self
            .inner
            .lifecycles
            .lock()
            .expect("lifecycle task lock poisoned")
            .accepting
        {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "snapshot deletion is unavailable during cleanup",
            ));
        }
        let removed = self.inner.snapshots.lock().await.remove(&snapshot_id);
        if removed.is_none() {
            return Err(RuntimeError::new(
                RuntimeErrorKind::NotFound,
                "snapshot not found",
            ));
        }
        release_snapshot_id(&self.inner, &snapshot_id);
        Ok(())
    }

    pub async fn fork_snapshot(&self, snapshot_id: &str) -> Result<String, RuntimeError> {
        self.fork_snapshot_with(
            snapshot_id,
            |container, image| async move { podman::create_container(&container, &image).await },
            |container, archive| async move {
                podman::restore_workspace(&container, archive.as_slice()).await
            },
            |container| async move { podman::remove_container(&container).await },
        )
        .await
    }

    async fn fork_snapshot_with<
        Create,
        CreateFuture,
        Restore,
        RestoreFuture,
        Remove,
        RemoveFuture,
    >(
        &self,
        snapshot_id: &str,
        create_container: Create,
        restore_workspace: Restore,
        remove_container: Remove,
    ) -> Result<String, RuntimeError>
    where
        Create: FnOnce(String, String) -> CreateFuture + Send + 'static,
        CreateFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
        Restore: FnOnce(String, Arc<Vec<u8>>) -> RestoreFuture + Send + 'static,
        RestoreFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
        Remove: FnOnce(String) -> RemoveFuture + Send + 'static,
        RemoveFuture: Future<Output = Result<(), RuntimeError>> + Send + 'static,
    {
        let snapshot = self
            .inner
            .snapshots
            .lock()
            .await
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| RuntimeError::new(RuntimeErrorKind::NotFound, "snapshot not found"))?;
        self.create_with(
            move |container, image| async move {
                create_container(container.clone(), image).await?;
                let archive = snapshot.archive();
                restore_workspace(container, archive).await
            },
            remove_container,
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

    fn allocate_snapshot_id(&self) -> Result<String, RuntimeError> {
        loop {
            let id = generate_opaque_id("snap-", "snapshot")?;
            if self
                .inner
                .issued_snapshot_ids
                .lock()
                .expect("snapshot ID lock poisoned")
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

    let snapshots: Vec<_> = inner
        .snapshots
        .lock()
        .await
        .drain()
        .map(|(_, snapshot)| snapshot)
        .collect();
    for snapshot in &snapshots {
        release_snapshot_id(&inner, &snapshot.summary().id);
    }
    drop(snapshots);

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

async fn finish_save_snapshot<Capture, CaptureFuture>(
    inner: Arc<StateInner>,
    pending: PendingSnapshot,
    result_sender: oneshot::Sender<Result<CreatedSnapshot, RuntimeError>>,
    capture_workspace: Capture,
) where
    Capture: FnOnce(String, usize) -> CaptureFuture,
    CaptureFuture: Future<Output = Result<Vec<u8>, RuntimeError>>,
{
    let PendingSnapshot {
        id,
        source_environment_id,
        source_container_name,
        slot,
        byte_reservation,
        capture_gate: _capture_gate,
    } = pending;
    let archive_limit = byte_reservation.num_permits();
    let archive = match capture_workspace(source_container_name, archive_limit).await {
        Ok(archive) => archive,
        Err(error) => {
            release_snapshot_id(&inner, &id);
            let _ = result_sender.send(Err(error));
            return;
        }
    };
    if archive.len() > archive_limit {
        release_snapshot_id(&inner, &id);
        let _ = result_sender.send(Err(RuntimeError::new(
            RuntimeErrorKind::Conflict,
            if archive_limit == MAX_SNAPSHOT_BYTES {
                "workspace snapshot exceeds the 64 MiB archive limit"
            } else {
                "workspace snapshot exceeds the remaining snapshot memory limit"
            },
        )));
        return;
    }
    if result_sender.is_closed() {
        release_snapshot_id(&inner, &id);
        return;
    }

    let summary = SnapshotSummary {
        id: id.clone(),
        source_environment_id,
        created_at_ms: now_ms(),
        archive_bytes: archive.len() as u64,
    };
    let snapshot = Arc::new(Snapshot::new(
        summary.clone(),
        archive,
        slot,
        byte_reservation,
    ));
    let inserted = {
        let mut snapshots = inner.snapshots.lock().await;
        match snapshots.entry(id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(snapshot.clone());
                true
            }
            Entry::Occupied(_) => false,
        }
    };
    if !inserted {
        release_snapshot_id(&inner, &id);
        let _ = result_sender.send(Err(RuntimeError::new(
            RuntimeErrorKind::Internal,
            "could not publish the reserved snapshot ID",
        )));
        return;
    }

    let (acknowledgement, acknowledged) = oneshot::channel();
    if result_sender
        .send(Ok(CreatedSnapshot {
            summary,
            acknowledgement,
        }))
        .is_err()
    {
        if take_snapshot(&inner, &id, &snapshot).await.is_some() {
            release_snapshot_id(&inner, &id);
        }
        return;
    }

    if acknowledged.await.is_err() && take_snapshot(&inner, &id, &snapshot).await.is_some() {
        release_snapshot_id(&inner, &id);
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
    environment.record_activity(ExecutionActivity::EnvironmentReady);
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

    if acknowledged.await.is_err() {
        if let Some(environment) = take_environment(&inner, &id, &environment).await {
            clean_unpublished_environment(inner, environment, remove_container).await;
        }
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
    let activity = Arc::new(ActivityLog::new());
    Arc::new(Environment {
        id,
        container_name,
        transcript: transcript.clone(),
        activity: activity.clone(),
        observation: Mutex::new(observation::ObservationState::new()),
        terminal_active: AtomicBool::new(false),
        terminal: terminal::TerminalHub::new(transcript, activity),
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

async fn take_snapshot(
    inner: &StateInner,
    id: &str,
    expected: &Arc<Snapshot>,
) -> Option<Arc<Snapshot>> {
    let mut snapshots = inner.snapshots.lock().await;
    if snapshots
        .get(id)
        .is_some_and(|snapshot| Arc::ptr_eq(snapshot, expected))
    {
        snapshots.remove(id)
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

fn release_snapshot_id(inner: &StateInner, id: &str) {
    inner
        .issued_snapshot_ids
        .lock()
        .expect("snapshot ID lock poisoned")
        .remove(id);
}

impl Environment {
    #[cfg(test)]
    pub(crate) fn for_test() -> Arc<Self> {
        let transcript = Arc::new(Transcript::new());
        let activity = Arc::new(ActivityLog::new());
        Arc::new(Self {
            id: "test".into(),
            container_name: "none".into(),
            transcript: transcript.clone(),
            activity: activity.clone(),
            observation: Mutex::new(observation::ObservationState::new()),
            terminal_active: AtomicBool::new(false),
            terminal: terminal::TerminalHub::new(transcript, activity),
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

    pub(crate) fn record_activity(&self, activity: ExecutionActivity) {
        self.activity.record(activity);
    }

    pub(crate) fn activity_snapshot(&self) -> (Vec<crate::activity::ExecutionEvent>, u64) {
        self.activity.snapshot()
    }

    pub(crate) fn activity(&self) -> &ActivityLog {
        &self.activity
    }

    pub(crate) fn observation_state(&self) -> &Mutex<observation::ObservationState> {
        &self.observation
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
    generate_opaque_id("env-", "environment")
}

fn generate_opaque_id(prefix: &str, kind: &str) -> Result<String, RuntimeError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| RuntimeError::internal(&format!("could not generate {kind} ID"), error))?;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(prefix.len() + bytes.len() * 2);
    id.push_str(prefix);
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

    #[test]
    fn snapshot_ids_are_opaque_128_bit_values() {
        let first = generate_opaque_id("snap-", "snapshot").unwrap();
        let second = generate_opaque_id("snap-", "snapshot").unwrap();

        for id in [&first, &second] {
            assert_eq!(id.len(), 37);
            assert!(id.starts_with("snap-"));
            assert!(id[5..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn snapshots_are_listed_and_release_owned_limits_on_delete() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "snapshot-source").await;
        let mut summaries = Vec::new();
        for byte in 0..MAX_SNAPSHOTS as u8 {
            summaries.push(
                runtime
                    .save_snapshot_with(
                        "snapshot-source",
                        move |_, _| async move { Ok(vec![byte]) },
                    )
                    .await
                    .unwrap(),
            );
        }

        let listed = runtime.list_snapshots().await;
        assert_eq!(listed.len(), MAX_SNAPSHOTS);
        assert!(listed.windows(2).all(|pair| {
            (pair[0].created_at_ms, &pair[0].id) <= (pair[1].created_at_ms, &pair[1].id)
        }));
        assert!(listed.iter().all(|summary| {
            summary.source_environment_id == "snapshot-source" && summary.archive_bytes == 1
        }));
        assert_eq!(runtime.inner.snapshot_slots.available_permits(), 0);
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES - MAX_SNAPSHOTS
        );

        let error = runtime
            .save_snapshot_with("snapshot-source", |_, _| async { Ok(vec![5]) })
            .await
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::Conflict);

        runtime
            .delete_snapshot(summaries[0].id.clone())
            .await
            .unwrap();
        assert_eq!(runtime.inner.snapshot_slots.available_permits(), 1);
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES - (MAX_SNAPSHOTS - 1)
        );
        assert!(
            !runtime
                .inner
                .issued_snapshot_ids
                .lock()
                .unwrap()
                .contains(&summaries[0].id)
        );

        runtime.cleanup_with(|_| async { Ok(()) }).await;
        assert!(runtime.inner.snapshots.lock().await.is_empty());
        assert_eq!(
            runtime.inner.snapshot_slots.available_permits(),
            MAX_SNAPSHOTS
        );
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES
        );
        assert_eq!(runtime.inner.snapshot_capture.available_permits(), 1);
        assert!(runtime.inner.issued_snapshot_ids.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_snapshot_captures_are_serialized() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "snapshot-memory-source").await;
        let (first_started_sender, first_started_receiver) = oneshot::channel();
        let (first_finish_sender, first_finish_receiver) = oneshot::channel();
        let first_runtime = runtime.clone();
        let first = tokio::spawn(async move {
            first_runtime
                .save_snapshot_with("snapshot-memory-source", move |_, limit| async move {
                    let _ = first_started_sender.send(());
                    let _ = first_finish_receiver.await;
                    assert_eq!(limit, MAX_SNAPSHOT_BYTES);
                    Ok(vec![1])
                })
                .await
        });
        first_started_receiver.await.unwrap();

        let (second_started_sender, mut second_started_receiver) = oneshot::channel();
        let (second_finish_sender, second_finish_receiver) = oneshot::channel();
        let second_runtime = runtime.clone();
        let second = tokio::spawn(async move {
            second_runtime
                .save_snapshot_with("snapshot-memory-source", move |_, limit| async move {
                    let _ = second_started_sender.send(limit);
                    let _ = second_finish_receiver.await;
                    Ok(vec![2])
                })
                .await
        });
        tokio::task::yield_now().await;
        assert!(matches!(
            second_started_receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(runtime.inner.snapshot_capture.available_permits(), 0);

        first_finish_sender.send(()).unwrap();
        first.await.unwrap().unwrap();
        assert_eq!(second_started_receiver.await.unwrap(), MAX_SNAPSHOT_BYTES);
        second_finish_sender.send(()).unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES - 2
        );
        runtime.cleanup_with(|_| async { Ok(()) }).await;
    }

    #[tokio::test]
    async fn capture_uses_the_remaining_total_budget_instead_of_requiring_sixty_four_mib() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "remaining-budget-source").await;
        let retained = runtime
            .inner
            .snapshot_bytes
            .clone()
            .try_acquire_many_owned((90 * 1024 * 1024) as u32)
            .unwrap();

        let summary = runtime
            .save_snapshot_with("remaining-budget-source", |_, limit| async move {
                assert_eq!(limit, 38 * 1024 * 1024);
                Ok(vec![1])
            })
            .await
            .unwrap();
        assert_eq!(summary.archive_bytes, 1);

        drop(retained);
        runtime.cleanup_with(|_| async { Ok(()) }).await;
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES
        );
    }

    #[tokio::test]
    async fn canceled_snapshot_capture_releases_all_ownership_after_capture_finishes() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "canceled-snapshot-source").await;
        let request_runtime = runtime.clone();
        let (started_sender, started_receiver) = oneshot::channel();
        let (finish_sender, finish_receiver) = oneshot::channel();
        let request = tokio::spawn(async move {
            request_runtime
                .save_snapshot_with("canceled-snapshot-source", move |_, _| async move {
                    let _ = started_sender.send(());
                    let _ = finish_receiver.await;
                    Ok(vec![1, 2, 3])
                })
                .await
        });

        started_receiver.await.unwrap();
        request.abort();
        let _ = request.await;
        assert_eq!(runtime.inner.snapshot_slots.available_permits(), 3);
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            64 * 1024 * 1024
        );
        finish_sender.send(()).unwrap();
        wait_until(|| runtime.inner.snapshot_slots.available_permits() == MAX_SNAPSHOTS).await;
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES
        );
        assert!(runtime.inner.snapshots.lock().await.is_empty());
        assert!(runtime.inner.issued_snapshot_ids.lock().unwrap().is_empty());
        runtime.cleanup_with(|_| async { Ok(()) }).await;
    }

    #[tokio::test]
    async fn unacknowledged_snapshot_delivery_is_removed() {
        let runtime = Runtime::new("unused".into());
        let id = runtime.allocate_snapshot_id().unwrap();
        let pending = PendingSnapshot {
            id: id.clone(),
            source_environment_id: "env-source".into(),
            source_container_name: "container-source".into(),
            slot: runtime
                .inner
                .snapshot_slots
                .clone()
                .try_acquire_owned()
                .unwrap(),
            byte_reservation: runtime
                .inner
                .snapshot_bytes
                .clone()
                .try_acquire_many_owned(MAX_SNAPSHOT_BYTES as u32)
                .unwrap(),
            capture_gate: runtime
                .inner
                .snapshot_capture
                .clone()
                .try_acquire_owned()
                .unwrap(),
        };
        let (result_sender, result_receiver) = oneshot::channel();
        let task = tokio::spawn(finish_save_snapshot(
            runtime.inner.clone(),
            pending,
            result_sender,
            |_, _| async { Ok(vec![1, 2, 3]) },
        ));

        let delivered = result_receiver.await.unwrap().unwrap();
        assert_eq!(delivered.summary.id, id);
        assert_eq!(runtime.inner.snapshots.lock().await.len(), 1);
        drop(delivered);
        task.await.unwrap();

        assert!(runtime.inner.snapshots.lock().await.is_empty());
        assert_eq!(
            runtime.inner.snapshot_slots.available_permits(),
            MAX_SNAPSHOTS
        );
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES
        );
        assert!(runtime.inner.issued_snapshot_ids.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fork_restores_archive_before_publishing_fresh_environment() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "fork-source").await;
        let snapshot = runtime
            .save_snapshot_with("fork-source", |_, _| async { Ok(vec![7, 8, 9]) })
            .await
            .unwrap();
        let (restored_sender, restored_receiver) = oneshot::channel();
        let fork_id = runtime
            .fork_snapshot_with(
                &snapshot.id,
                |_, _| async { Ok(()) },
                move |container, archive| async move {
                    let _ = restored_sender.send((container, archive.as_ref().clone()));
                    Ok(())
                },
                |_| async { Ok(()) },
            )
            .await
            .unwrap();

        let (container, archive) = restored_receiver.await.unwrap();
        assert!(container.starts_with("clannon-"));
        assert_eq!(archive, vec![7, 8, 9]);
        assert_ne!(fork_id, "fork-source");
        let fork = runtime.find(&fork_id).await.unwrap();
        assert!(fork.transcript_snapshot().await.is_empty());
        assert_eq!(
            fork.activity_snapshot().0[0].activity,
            ExecutionActivity::EnvironmentReady
        );
        assert_eq!(fork.activity_snapshot().0.len(), 1);

        runtime.cleanup_with(|_| async { Ok(()) }).await;
    }

    #[tokio::test]
    async fn deleting_snapshot_during_fork_keeps_bytes_reserved_through_restore() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "delete-during-fork-source").await;
        let snapshot = runtime
            .save_snapshot_with("delete-during-fork-source", |_, _| async {
                Ok(vec![4, 5, 6])
            })
            .await
            .unwrap();
        let (restore_started_sender, restore_started_receiver) = oneshot::channel();
        let (finish_restore_sender, finish_restore_receiver) = oneshot::channel();
        let fork_runtime = runtime.clone();
        let snapshot_id = snapshot.id.clone();
        let fork = tokio::spawn(async move {
            fork_runtime
                .fork_snapshot_with(
                    &snapshot_id,
                    |_, _| async { Ok(()) },
                    move |_, archive| async move {
                        let _ = restore_started_sender.send(archive.len());
                        let _ = finish_restore_receiver.await;
                        Ok(())
                    },
                    |_| async { Ok(()) },
                )
                .await
        });

        assert_eq!(restore_started_receiver.await.unwrap(), 3);
        runtime.delete_snapshot(snapshot.id).await.unwrap();
        assert!(runtime.inner.snapshots.lock().await.is_empty());
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES - 3,
            "the in-flight restore must continue owning the archive bytes"
        );

        finish_restore_sender.send(()).unwrap();
        fork.await.unwrap().unwrap();
        assert_eq!(
            runtime.inner.snapshot_bytes.available_permits(),
            MAX_RETAINED_SNAPSHOT_BYTES
        );
        assert_eq!(
            runtime.inner.snapshot_slots.available_permits(),
            MAX_SNAPSHOTS
        );
        runtime.cleanup_with(|_| async { Ok(()) }).await;
    }

    #[tokio::test]
    async fn failed_snapshot_restore_removes_the_unpublished_fork() {
        let runtime = Runtime::new("unused".into());
        insert_test_environment(&runtime, "restore-source").await;
        let snapshot = runtime
            .save_snapshot_with("restore-source", |_, _| async { Ok(vec![1]) })
            .await
            .unwrap();
        let (removed_sender, removed_receiver) = oneshot::channel();
        let error = runtime
            .fork_snapshot_with(
                &snapshot.id,
                |_, _| async { Ok(()) },
                |_, _| async {
                    Err(RuntimeError::new(
                        RuntimeErrorKind::BadGateway,
                        "test restore failed",
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
        assert_eq!(runtime.inner.environments.lock().await.len(), 1);
        assert_eq!(runtime.inner.environment_slots.available_permits(), 3);
        runtime.cleanup_with(|_| async { Ok(()) }).await;
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
        let activity = Arc::new(ActivityLog::new());
        let environment = Environment {
            id: "test".into(),
            container_name: "none".into(),
            transcript: transcript.clone(),
            activity: activity.clone(),
            observation: Mutex::new(observation::ObservationState::new()),
            terminal_active: AtomicBool::new(false),
            terminal: terminal::TerminalHub::new(transcript, activity),
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
    async fn successful_create_records_ready_before_delivery() {
        let runtime = Runtime::new("unused".into());
        let id = runtime
            .create_with(|_, _| async { Ok(()) }, |_| async { Ok(()) })
            .await
            .unwrap();

        let environment = runtime.find(&id).await.unwrap();
        let (events, omitted) = environment.activity_snapshot();
        assert_eq!(omitted, 0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[0].activity, ExecutionActivity::EnvironmentReady);

        runtime.cleanup_with(|_| async { Ok(()) }).await;
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
        let environments = runtime.inner.environments.lock().await;
        assert_eq!(environments.len(), 1);
        assert!(
            environments
                .values()
                .next()
                .unwrap()
                .activity_snapshot()
                .0
                .is_empty(),
            "an unknown failed create is not evidence that the environment became ready"
        );
        drop(environments);
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
