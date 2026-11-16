//! Bounded projection rebuild over generated original-library event history.
use async_trait::async_trait;
use statechronicle::accumulator::sparse_merkle::StateUpdate;
use statechronicle::domain::ids::CommitId;
use statechronicle::index::rebuild::{
    CheckpointStore, ProjectionSink, rebuild_projections_chunk, rebuild_projections_resumable,
};
use statechronicle::{Event, StateAccumulator, StateKey, StateProjection};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default, Clone)]
struct Storage {
    projections: Arc<Mutex<BTreeMap<(String, String), StateProjection>>>,
    checkpoint: Arc<Mutex<Option<usize>>>,
}

#[async_trait]
impl ProjectionSink for Storage {
    async fn upsert_projection(&self, projection: &StateProjection) -> Result<(), String> {
        let key = (
            projection.tenant_id.0.clone(),
            projection.resource_id.0.clone(),
        );
        let mut rows = self.projections.lock().unwrap();
        if let Some(old) = rows.get(&key) {
            assert!(projection.version >= old.version);
            if projection.version == old.version {
                assert_eq!(projection, old);
            }
        }
        rows.insert(key, projection.clone());
        Ok(())
    }
}

#[async_trait]
impl CheckpointStore for Storage {
    async fn load(&self, key: &str) -> Result<Option<usize>, String> {
        assert_eq!(key, "bench");
        Ok(*self.checkpoint.lock().unwrap())
    }
    async fn save(&self, key: &str, next: usize) -> Result<(), String> {
        assert_eq!(key, "bench");
        *self.checkpoint.lock().unwrap() = Some(next);
        Ok(())
    }
}

pub async fn resume_and_check(events: &[(Event, CommitId)], expected_root: &[u8; 32]) {
    assert_eq!(events.len(), 256);
    let first = Storage::default();
    let checkpoint = rebuild_projections_chunk(events, 0, 32, &first)
        .await
        .unwrap();
    assert_eq!(checkpoint.next_event, 32);
    first.save("bench", checkpoint.next_event).await.unwrap();
    // New handles share the persisted in-memory data. This exercises resume
    // semantics, not a process crash or durable database recovery.
    let reopened = first.clone();
    let written = rebuild_projections_resumable(events, "bench", 32, &reopened, &reopened)
        .await
        .unwrap();
    assert_eq!(written, 224);
    assert_eq!(reopened.load("bench").await.unwrap(), Some(256));
    let rows = reopened.projections.lock().unwrap();
    assert_eq!(rows.len(), 256);
    let mut tree = StateAccumulator::empty();
    let updates: Vec<_> = rows
        .values()
        .map(|row| {
            let key = StateKey::for_resource(&row.tenant_id.0, &row.resource_id.0);
            StateUpdate::new(key, *row.state_hash.as_bytes())
        })
        .collect();
    tree.insert_batch(&updates).unwrap();
    assert_eq!(tree.root().as_bytes(), expected_root);
}
