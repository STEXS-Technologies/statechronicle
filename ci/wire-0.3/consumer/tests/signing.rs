use chrono::{TimeZone, Utc};
use ed25519_dalek::{Signer, SigningKey};
use statechronicle_commit::sign::{
    CommitSigner, sign_commit, sign_commit_with_signer, verify_commit,
};
use statechronicle_core::{digest::ContentDigest, signature::Signature};
use statechronicle_domain::{
    commit::{Commit, CommitScope, ProfileId},
    ids::CommitId,
    intent::KeyId,
    subject::SubjectId,
    tenant::TenantId,
};

struct CallerSigner(SigningKey);
impl CommitSigner for CallerSigner {
    fn sign_bytes(
        &self,
        _key_id: &KeyId,
        canonical: &[u8],
    ) -> Result<Signature, statechronicle_commit::error::CommitError> {
        Ok(Signature::from_bytes(self.0.sign(canonical).to_bytes()))
    }
}

fn fixture_commit() -> Commit {
    let digest = ContentDigest::new([7; 32]);
    Commit::new(
        CommitScope::tenant(TenantId(String::from("consumer.example"))),
        CommitId::new(String::from("cmt_consumer_001")).unwrap(),
        None,
        1,
        0,
        digest.clone(),
        digest.clone(),
        digest,
        Utc.timestamp_opt(1_780_000_000, 0).unwrap(),
        SubjectId(String::from("service:consumer.example")),
        ProfileId::new(String::from("profile:test")).unwrap(),
    )
}

#[test]
fn published_default_ed25519_api_signs_and_verifies() {
    let key = SigningKey::from_bytes(&[0x42; 32]);
    let key_id = KeyId::new(String::from("did:key:consumer#commit")).unwrap();
    let signed = sign_commit(&fixture_commit(), &key, key_id).unwrap();
    verify_commit(&signed, &key.verifying_key()).unwrap();
}

#[test]
fn published_caller_commit_signer_api_signs_and_verifies() {
    let key = SigningKey::from_bytes(&[0x24; 32]);
    let key_id = KeyId::new(String::from("did:key:consumer#kms")).unwrap();
    let signed =
        sign_commit_with_signer(&fixture_commit(), key_id, &CallerSigner(key.clone())).unwrap();
    verify_commit(&signed, &key.verifying_key()).unwrap();
}
