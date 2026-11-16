//! Deterministic error/port-order characterization of four compatibility methods.
//! Run against frozen original production, the extraction control and candidate.
use async_trait::async_trait;
use serde_json::{Value, json};
use statechronicle::domain::ids::{CommitId, EventId, SnapshotId};
use statechronicle::domain::proof::{
    NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle::domain::resource_state::{FungibleBalanceState, ResourceState};
use statechronicle::ports::commit_store::{CanonicalHead, CommitStore, CommitStoreError};
use statechronicle::ports::proof_index::{ProofIndex, ProofIndexError};
use statechronicle::ports::snapshot_store::{SnapshotStore, SnapshotStoreError};
use statechronicle::ports::state_index::{StateIndex, StateIndexError};
use statechronicle::proof::service::{ProofPorts, ProofService};
use statechronicle::{Commit, ResourceId, Signed, StateKey, StateProjection, SubjectId, TenantId};
use std::sync::{Arc, Mutex};

struct Reads {
    signed: Signed<Commit>,
    store: usize,
    head: usize,
    calls: Option<Arc<Mutex<Vec<String>>>>,
}
impl Reads {
    fn record(&self, value: impl FnOnce() -> String) {
        if let Some(calls) = &self.calls {
            calls.lock().unwrap().push(value());
        }
    }
}
#[async_trait]
impl CommitStore for Reads {
    async fn put_commit(&self, _: &TenantId, _: &Signed<Commit>) -> Result<(), CommitStoreError> {
        panic!("unexpected write")
    }
    async fn commit_by_sequence(
        &self,
        _: &TenantId,
        _: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        panic!("unexpected sequence read")
    }
    async fn commit_by_id(
        &self,
        tenant: &TenantId,
        id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        self.record(|| format!("commit:{}:{}", tenant.0, id.0));
        match self.store {
            0 => Ok(Some(self.signed.clone())),
            1 => Ok(None),
            _ => Err(CommitStoreError::Unavailable(
                "fixture lookup unavailable".into(),
            )),
        }
    }
    async fn canonical_head(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<CanonicalHead>, CommitStoreError> {
        self.record(|| format!("head:{}", tenant.0));
        let mut head = CanonicalHead {
            commit_id: self.signed.body.commit_id.clone(),
            sequence: self.signed.body.sequence,
            state_root: self.signed.body.next_state_root.clone(),
        };
        match self.head {
            1 => return Ok(None),
            2 => {
                return Err(CommitStoreError::Unavailable(
                    "fixture head unavailable".into(),
                ));
            }
            3 => head.commit_id = CommitId::new("cmt_other".into()).unwrap(),
            4 => head.sequence += 1,
            5 => head.state_root = statechronicle::core::digest::hash_bytes(b"wrong head root"),
            6 => {
                head.sequence += 1;
                head.state_root = statechronicle::core::digest::hash_bytes(b"wrong head root");
            }
            _ => (),
        }
        Ok(Some(head))
    }
}
struct Unused;

#[async_trait]
impl ProofIndex for Unused {
    async fn get_state_proof(
        &self,
        _: &TenantId,
        _: &ResourceId,
        _: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        panic!("unexpected proof lookup")
    }
    async fn get_ownership_proof(
        &self,
        _: &TenantId,
        _: &ResourceId,
        _: &SubjectId,
        _: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        panic!("unexpected ownership lookup")
    }
    async fn get_inclusion_proof(
        &self,
        _: &TenantId,
        _: &EventId,
        _: &CommitId,
    ) -> Result<Option<SparseMerkleProof>, ProofIndexError> {
        panic!("unexpected inclusion lookup")
    }
    async fn get_non_membership_proof(
        &self,
        _: &TenantId,
        _: &ResourceId,
        _: StateKey,
        _: Option<&CommitId>,
    ) -> Result<Option<NonMembershipProofBundle>, ProofIndexError> {
        panic!("unexpected absence lookup")
    }
}

#[async_trait]
impl SnapshotStore for Unused {
    async fn put_snapshot(
        &self,
        _: &TenantId,
        _: &SnapshotId,
        _: Vec<u8>,
    ) -> Result<(), SnapshotStoreError> {
        panic!("verification must not write snapshots")
    }
    async fn get_snapshot(
        &self,
        _: &TenantId,
        _: &SnapshotId,
    ) -> Result<Option<Vec<u8>>, SnapshotStoreError> {
        panic!("unexpected snapshot lookup")
    }
}

#[async_trait]
impl StateIndex for Unused {
    async fn get_state(
        &self,
        _: &TenantId,
        _: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        panic!("unexpected projection read")
    }
    async fn get_subject_state(
        &self,
        _: &TenantId,
        _: &SubjectId,
        _: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        panic!("unexpected subject read")
    }
}
fn service(
    signed: &Signed<Commit>,
    store: usize,
    head: usize,
    calls: Option<Arc<Mutex<Vec<String>>>>,
) -> ProofService {
    ProofService::new(ProofPorts {
        proof_index: Box::new(Unused),
        state_index: Box::new(Unused),
        snapshot_store: Box::new(Unused),
        commit_store: Box::new(Reads {
            signed: signed.clone(),
            store,
            head,
            calls,
        }),
    })
}
async fn verify(
    service: &ProofService,
    method: usize,
    proof: &ResourceStateProof,
    key: &StateKey,
    trusted: &ed25519_dalek::VerifyingKey,
) -> Result<(), statechronicle::proof::error::ProofError> {
    match method {
        0 => service.verify(proof, trusted).await,
        1 => service.verify_with_key(proof, trusted, key).await,
        2 => service.verify_canonical(proof, trusted).await,
        _ => service.verify_with_key_canonical(proof, trusted, key).await,
    }
}
fn main() {
    let input: Value = serde_json::from_slice(
        &std::fs::read(std::env::args().nth(1).expect("wire corpus path")).unwrap(),
    )
    .unwrap();
    let signed: Signed<Commit> = serde_json::from_value(input["signed_commit"].clone()).unwrap();
    let original: ResourceStateProof =
        serde_json::from_value(input["state_proof"].clone()).unwrap();
    let trusted = ed25519_dalek::SigningKey::from_bytes(&[42; 32]).verifying_key();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let key = StateKey::for_resource(&original.tenant_id.0, &original.resource_id.0);
    let mut trace = Vec::new();
    for method in 0..4 {
        for tamper in 0..14 {
            for store in 0..3 {
                for head in 0..7 {
                    for wrong_key in 0..2 {
                        let mut proof = original.clone();
                        let mut stored = signed.clone();
                        match tamper {
                            1 => proof.schema = "unsupported.schema".into(),
                            2 => proof.state_inclusion_proof.kind = "unsupported.kind".into(),
                            3 => {
                                proof.state_inclusion_proof.leaf_hash =
                                    statechronicle::core::digest::hash_bytes(b"wrong leaf")
                            }
                            4 => {
                                let ResourceState::UniqueAsset(state) = &mut proof.claimed_state
                                else {
                                    panic!("asset fixture")
                                };
                                state.owner = SubjectId("account:other".into());
                            }
                            5 => proof.tenant_id = TenantId("acme.game.other".into()),
                            6 => proof.resource_id = ResourceId("asset:other".into()),
                            7 => proof.commit.sequence += 1,
                            8 => {
                                proof.commit.state_root =
                                    statechronicle::core::digest::hash_bytes(b"wrong proof root")
                            }
                            9 | 10 => {
                                proof.claimed_state =
                                    ResourceState::FungibleBalance(FungibleBalanceState {
                                        subject: SubjectId(
                                            if tamper == 9 { "" } else { "account:holder" }.into(),
                                        ),
                                        balance: statechronicle::Amount::ZERO,
                                        unit: "COIN".into(),
                                    })
                            }
                            11 => proof.state_inclusion_proof.path.clear(),
                            12 => {
                                stored.signature.sig =
                                    statechronicle::Signature::from_bytes([0; 64])
                            }
                            13 => {
                                proof.commit.signature.sig =
                                    statechronicle::Signature::from_bytes([0; 64])
                            }
                            _ => (),
                        }
                        let calls = Arc::new(Mutex::new(Vec::new()));
                        let service = service(&stored, store, head, Some(calls.clone()));
                        let supplied_key = if wrong_key == 0 {
                            key
                        } else {
                            StateKey::for_resource(&original.tenant_id.0, "asset:other")
                        };
                        let result = runtime.block_on(verify(
                            &service,
                            method,
                            &proof,
                            &supplied_key,
                            &trusted,
                        ));
                        if tamper == 0 && store == 0 && head == 0 && wrong_key == 0 {
                            assert!(result.is_ok(), "valid fixture failed: {result:?}");
                        }
                        if tamper == 9 && (method == 0 || method == 2) {
                            assert!(matches!(
                                &result,
                                Err(statechronicle::proof::error::ProofError::InvalidState(_))
                            ));
                            assert!(calls.lock().unwrap().is_empty());
                        }
                        let outcome = result.map(|()| json!({"ok": true})).unwrap_or_else(
                            |e| json!({"debug": format!("{e:?}"), "display": e.to_string()}),
                        );
                        trace.push(json!({"case": [method,tamper,store,head,wrong_key], "outcome": outcome, "calls": *calls.lock().unwrap()}));
                    }
                }
            }
        }
    }
    assert_eq!(trace.len(), 2352);
    println!("{}", serde_json::to_string(&trace).unwrap());
}
