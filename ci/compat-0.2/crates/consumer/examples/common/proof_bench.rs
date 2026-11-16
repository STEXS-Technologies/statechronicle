//! Fixed read adapters for measuring the original public proof service.
//! Unused ports panic so a changed workload cannot silently add fake work.
use async_trait::async_trait;
use statechronicle::domain::ids::{CommitId, EventId, SnapshotId};
use statechronicle::domain::proof::{
    NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle::ports::commit_store::{CanonicalHead, CommitStore, CommitStoreError};
use statechronicle::ports::proof_index::{ProofIndex, ProofIndexError};
use statechronicle::ports::snapshot_store::{SnapshotStore, SnapshotStoreError};
use statechronicle::proof::service::{ProofPorts, ProofService};
use statechronicle::{Commit, ResourceId, Signed, StateKey, SubjectId, TenantId};

struct FixedCommit {
    tenant: TenantId,
    signed: Signed<Commit>,
    head: CanonicalHead,
}

#[async_trait]
impl CommitStore for FixedCommit {
    async fn put_commit(&self, _: &TenantId, _: &Signed<Commit>) -> Result<(), CommitStoreError> {
        panic!("verification must not write commits")
    }
    async fn commit_by_id(
        &self,
        tenant: &TenantId,
        id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        assert_eq!(tenant, &self.tenant);
        assert_eq!(id, &self.signed.body.commit_id);
        Ok(Some(self.signed.clone()))
    }
    async fn commit_by_sequence(
        &self,
        _: &TenantId,
        _: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        panic!("unexpected sequence lookup")
    }
    async fn canonical_head(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<CanonicalHead>, CommitStoreError> {
        assert_eq!(tenant, &self.tenant);
        Ok(Some(self.head.clone()))
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

pub fn service(tenant: TenantId, signed: &Signed<Commit>, stale: bool) -> ProofService {
    let head = CanonicalHead {
        commit_id: if stale {
            CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF6")).unwrap()
        } else {
            signed.body.commit_id.clone()
        },
        sequence: signed.body.sequence + u64::from(stale),
        state_root: signed.body.next_state_root.clone(),
    };
    ProofService::new(ProofPorts {
        proof_index: Box::new(Unused),
        state_index: Box::new(crate::common::FakeStateIndex::default()),
        commit_store: Box::new(FixedCommit {
            tenant,
            signed: signed.clone(),
            head,
        }),
        snapshot_store: Box::new(Unused),
    })
}
