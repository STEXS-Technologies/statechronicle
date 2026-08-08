#![no_main]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::signature::Signature;
use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::{KeyId, Operation, SignatureAlg, SignatureBlock};
use statechronicle_domain::proof::{CommitRef, EventRef, ResourceStateProof, SparseMerkleProof};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::trade::{
    TradeProof, TradeProofLeg, TradeSide, TradeStatus, TradeSummary, TradeValueLeg,
};
use statechronicle_proof::trade::verify_trade_proof;

// `verify_trade_proof` is total over arbitrary proofs: any structurally
// arbitrary trade proof plus an arbitrary commits-by-tenant map must return a
// `Result`, never panic. The byte stream derives the proof's legs, summary,
// state proofs, and the signed commits, so every fail-closed path (unsupported
// schema, missing tenant key, commit-ref/signature mismatch, structural
// checks) is reachable.
fuzz_target!(|data: &[u8]| {
    let Some(proof) = proof_from_bytes(data) else {
        return;
    };
    let commits = commits_by_tenant(data);
    let _ = verify_trade_proof(&proof, &commits);
});

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn tenant_name(data: &[u8]) -> &'static str {
    if data.first().copied().unwrap_or(0) % 2 == 0 {
        "acme.game.alpha"
    } else {
        "acme.game.beta"
    }
}

fn digest(data: &[u8], seed: u8) -> ContentDigest {
    let mut bytes = [0u8; 32];
    for (index, byte) in data.iter().take(32).enumerate() {
        bytes[index] = byte.wrapping_add(seed);
    }
    ContentDigest::new(bytes)
}

fn signature_block(data: &[u8]) -> SignatureBlock {
    let mut sig = [0u8; 64];
    for (index, byte) in data.iter().take(64).enumerate() {
        sig[index] = *byte;
    }
    SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#fuzz")).unwrap(),
        sig: Signature::from_bytes(sig),
    }
}

fn commit_ref(data: &[u8], _tenant: &str) -> CommitRef {
    CommitRef {
        commit_id: CommitId::new(format!("cmt_{:08}", data.first().copied().unwrap_or(1))).unwrap(),
        sequence: data.get(1).copied().unwrap_or(0) as u64,
        state_root: digest(data, 1),
        signature: signature_block(data),
    }
}

fn state_proof(data: &[u8], tenant: &str) -> ResourceStateProof {
    let path_len = (data.get(2).copied().unwrap_or(0) % 40) as usize;
    let mut path = Vec::new();
    for index in 0..path_len {
        let mut bytes = [0u8; 32];
        for (i, byte) in data.iter().skip(index).take(32).enumerate() {
            bytes[i] = byte.wrapping_add(index as u8);
        }
        path.push(ContentDigest::new(bytes));
    }
    ResourceStateProof::new(
        TenantId(String::from(tenant)),
        ResourceId(String::from(
            if data.get(3).copied().unwrap_or(0) % 2 == 0 {
                "asset:sword"
            } else {
                "asset:shield"
            },
        )),
        serde_json::json!({ "owner": "account:example:player_456", "status": "active" }),
        commit_ref(data, tenant),
        SparseMerkleProof::new(path, digest(data, 4)),
        EventRef {
            event_id: EventId::new(String::from("evt_00000000000000000001")).unwrap(),
            operation: Operation::from_static(if data.get(5).copied().unwrap_or(0) % 2 == 0 {
                "trade.settle"
            } else {
                "trade.unlock"
            }),
        },
        None,
    )
}

fn proof_from_bytes(data: &[u8]) -> Option<TradeProof> {
    let tenant = tenant_name(data);
    let trade_id = format!("trade_{:03}", data.get(1).copied().unwrap_or(0) % 7);
    let side = TradeSide {
        tenant: TenantId(String::from(tenant)),
        settle_assets: vec![ResourceId(String::from("asset:sword"))],
        from_owner: String::from("account:example:player_123"),
        to_owner: String::from("account:example:player_456"),
        settle_commit: commit_ref(data, tenant),
        settle_event_ids: vec![EventId::new(String::from("evt_00000000000000000001")).ok()?],
    };
    let summary = TradeSummary {
        trade_id: trade_id.clone(),
        status: TradeStatus::Settled,
        sides: vec![side],
        value_legs: Vec::<TradeValueLeg>::new(),
    };
    let leg = TradeProofLeg {
        tenant: TenantId(String::from(tenant)),
        commit: commit_ref(data, tenant),
        state_proofs: vec![state_proof(data, tenant)],
    };
    Some(TradeProof {
        schema: String::from(statechronicle_domain::trade::TRADE_PROOF_SCHEMA),
        trade_id,
        summary,
        legs: vec![leg],
    })
}

fn commits_by_tenant(
    data: &[u8],
) -> BTreeMap<String, (Signed<Commit>, ed25519_dalek::VerifyingKey)> {
    let tenant = tenant_name(data);
    let commit = Commit::new(
        CommitScope::tenant(TenantId(String::from(tenant))),
        CommitId::new(format!("cmt_{:08}", data.first().copied().unwrap_or(1))).unwrap(),
        None,
        data.get(1).copied().unwrap_or(0) as u64,
        1,
        hash_bytes(b"event-root"),
        hash_bytes(b"previous-root"),
        digest(data, 1),
        now(),
        SubjectId(String::from("service:statechronicle.example.net")),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    );
    let signed = Signed::new(commit, signature_block(data));
    let seed = ed25519_dalek::SigningKey::from_bytes(&[42u8; 32]);
    let mut map = BTreeMap::new();
    map.insert(String::from(tenant), (signed, seed.verifying_key()));
    map
}
