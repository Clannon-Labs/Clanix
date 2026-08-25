use std::sync::Arc;

use serde::Serialize;
use tokio::sync::OwnedSemaphorePermit;

pub(crate) const MAX_SNAPSHOTS: usize = 4;
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_RETAINED_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SnapshotSummary {
    pub id: String,
    pub source_environment_id: String,
    pub created_at_ms: u128,
    pub archive_bytes: u64,
}

pub(crate) struct Snapshot {
    summary: SnapshotSummary,
    archive: Arc<Vec<u8>>,
    _slot: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

impl Snapshot {
    pub(crate) fn new(
        summary: SnapshotSummary,
        archive: Vec<u8>,
        slot: OwnedSemaphorePermit,
        mut byte_reservation: OwnedSemaphorePermit,
    ) -> Self {
        let retained_bytes = byte_reservation
            .split(archive.len())
            .expect("capture cannot exceed its reserved snapshot bytes");
        drop(byte_reservation);
        Self {
            summary,
            archive: Arc::new(archive),
            _slot: slot,
            _bytes: retained_bytes,
        }
    }

    pub(crate) fn summary(&self) -> SnapshotSummary {
        self.summary.clone()
    }

    pub(crate) fn archive(&self) -> Arc<Vec<u8>> {
        self.archive.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Semaphore;

    #[test]
    fn snapshot_releases_unused_byte_reservation() {
        let byte_budget = Arc::new(Semaphore::new(100));
        let slots = Arc::new(Semaphore::new(1));
        let byte_reservation = byte_budget.clone().try_acquire_many_owned(100).unwrap();
        let slot = slots.clone().try_acquire_owned().unwrap();

        let snapshot = Snapshot::new(
            SnapshotSummary {
                id: "snap-test".into(),
                source_environment_id: "env-test".into(),
                created_at_ms: 1,
                archive_bytes: 25,
            },
            vec![0; 25],
            slot,
            byte_reservation,
        );

        assert_eq!(byte_budget.available_permits(), 75);
        assert_eq!(slots.available_permits(), 0);
        drop(snapshot);
        assert_eq!(byte_budget.available_permits(), 100);
        assert_eq!(slots.available_permits(), 1);
    }
}
