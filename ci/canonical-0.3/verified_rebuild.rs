//! Fresh external application imports only the public umbrella and runtime ports.
mod common;
use async_trait::async_trait;
use statechronicle::commit::{
    batch::CommitBatch,
    builder::CommitBuilder,
    roots::compute_state_root,
    sign::{Ed25519CommitVerifier, sign_commit},
};
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::domain::{
    commit::CommitScope,
    event::StateCommitment,
    resource_state::{ResourceState, UniqueAssetState},
    status::Status,
};
use statechronicle::index::rebuild::{
    CheckpointStore, ProjectionSink,
    verified::{HistoryPin, HistoryWitness, verify_history},
};
use statechronicle::{
    CommitId, ContentDigest, Event, EventId, IntentId, Operation, ProfileId, ResourceId,
    StateProjection, SubjectId,
};
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
struct RebuildDatabase {
    projections: Mutex<Vec<StateProjection>>,
    checkpoints: Mutex<BTreeMap<String, usize>>,
}
#[async_trait]
impl ProjectionSink for RebuildDatabase {
    async fn upsert_projection(&self, projection: &StateProjection) -> Result<(), String> {
        let mut rows = self.projections.lock().unwrap();
        if let Some(old) = rows.iter_mut().find(|old| {
            old.tenant_id == projection.tenant_id && old.resource_id == projection.resource_id
        }) {
            if old.version > projection.version {
                return Ok(());
            }
            if old.version == projection.version && old != projection {
                return Err("conflicting version".into());
            }
            *old = projection.clone();
        } else {
            rows.push(projection.clone());
        }
        Ok(())
    }
}
#[async_trait]
impl CheckpointStore for RebuildDatabase {
    async fn load(&self, key: &str) -> Result<Option<usize>, String> {
        Ok(self.checkpoints.lock().unwrap().get(key).copied())
    }
    async fn save(&self, key: &str, offset: usize) -> Result<(), String> {
        self.checkpoints.lock().unwrap().insert(key.into(), offset);
        Ok(())
    }
}
#[tokio::test]
async fn pinned_signed_history_rebuilds_through_only_the_public_facade() {
    let commitment = |owner: &str, status: &'static str, version| {
        let state = ResourceState::UniqueAsset(UniqueAssetState {
            owner: SubjectId(owner.into()),
            status: Status::from_static(status),
            trade_id: None,
        });
        StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        }
    };
    let event = Event::new(
        common::tenant(),
        EventId::new("evt_verified_rebuild".into()).unwrap(),
        IntentId::new("int_verified_rebuild".into()).unwrap(),
        Operation::from_static("asset.mint"),
        ResourceId("asset:rebuild".into()),
        SubjectId("account:alice".into()),
        commitment("", "unborn", 0),
        commitment("account:alice", "active", 1),
        None,
        common::executor_subject(),
        common::fixed_now(),
    );
    let mut batch = CommitBatch::new(CommitScope::tenant(common::tenant()));
    batch.add_event(event.clone()).unwrap();
    let body = CommitBuilder::builder()
        .scope(batch.scope().clone())
        .sequence(1)
        .executor(common::executor_subject())
        .profile(ProfileId::new("statechronicle.profile.resource.v0".into()).unwrap())
        .created_at(common::fixed_now())
        .build(
            &batch,
            ContentDigest::new(*compute_state_root(&[]).unwrap().as_bytes()),
            &[],
            || Ok(CommitId::new("cmt_verified_rebuild".into()).unwrap()),
        )
        .unwrap();
    // In this test the trusted producer fixes the pin before provider input.
    let expected = HistoryPin {
        tenant: common::tenant(),
        tip_digest: canonicalize_and_digest(&body).unwrap(),
    };
    let signed = sign_commit(&body, &common::fixed_key(), common::key_id()).unwrap();
    let verifier =
        Ed25519CommitVerifier::new(|commit: &statechronicle::Signed<statechronicle::Commit>| {
            if commit.body.scope.tenant_id != Some(common::tenant())
                || commit.signature.key_id != common::key_id()
            {
                return Err("untrusted historical signer".into());
            }
            Ok(common::fixed_key().verifying_key())
        });
    let events = vec![event.clone()];
    let history = verify_history(
        &[HistoryWitness {
            commit: &signed,
            events: &events,
        }],
        &expected,
        &verifier,
    )
    .unwrap();
    let database = RebuildDatabase::default();
    history
        .rebuild_resumable("recovery", 1, &database, &database)
        .await
        .unwrap();
    let rows = database.projections.lock().unwrap();
    assert_eq!(rows.len(), 1);
    let row = rows.first().unwrap();
    assert_eq!(row.state, event.after.state);
    assert_eq!(row.last_commit_id, body.commit_id);
    assert_eq!(
        database
            .checkpoints
            .lock()
            .unwrap()
            .get(&history.checkpoint_key("recovery").unwrap()),
        Some(&1)
    );
}
