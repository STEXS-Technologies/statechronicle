#![no_main]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::hash_bytes;
use statechronicle_core::signature::Signature;
use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::{
    Intent, KeyId, Nonce, Operation, SignatureAlg, SignatureBlock,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_index::build::{IngestBatch, apply};

// `apply` is total over arbitrary batches: any derivation of trade events,
// value pairs, settle intents, and a committing signed commit must return a
// `Result`, never panic. The byte stream decides which events and intents land
// in the batch, so every fail-closed path (missing trade_id, non-tenant commit,
// malformed value leg) is reachable.
fuzz_target!(|data: &[u8]| {
    let Some(batch) = batch_from_bytes(data) else {
        return;
    };
    let mut state: BTreeMap<String, statechronicle_domain::trade::TradeRecord> = BTreeMap::new();
    let _ = apply(&mut state, &batch);
});

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
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

fn event(id: usize, tenant: &str, op: &str, trade_id: &str) -> Event {
    let before = match op {
        "trade.settle" | "trade.unlock" => {
            serde_json::json!({ "owner": "alice", "status": "trade_held", "trade_id": trade_id })
        }
        _ => serde_json::json!({ "owner": "alice", "status": "active" }),
    };
    let after = match op {
        "trade.lock" => {
            serde_json::json!({ "owner": "alice", "status": "trade_held", "trade_id": trade_id })
        }
        "trade.settle" => serde_json::json!({ "owner": "bob", "status": "active" }),
        _ => serde_json::json!({ "owner": "alice", "status": "active" }),
    };
    Event::new(
        TenantId(String::from(tenant)),
        EventId::new(format!("evt_{id:020}")).ok().unwrap(),
        IntentId::new(format!("int_{id:08}")).ok().unwrap(),
        Operation::new(String::from(op)).unwrap(),
        ResourceId(String::from("asset:sword")),
        SubjectId(String::from("account:example:player")),
        StateCommitment {
            version: 1,
            state_hash: hash_bytes(b"before"),
            state: before,
        },
        StateCommitment {
            version: 2,
            state_hash: hash_bytes(b"after"),
            state: after,
        },
        None,
        SubjectId(String::from("service:statechronicle.example.net")),
        now(),
    )
}

fn batch_from_bytes(data: &[u8]) -> Option<IngestBatch> {
    let tenant = tenant_name(data);
    let trade_id = format!("trade_{:03}", data.get(1).copied().unwrap_or(0) % 7);
    let mut events = Vec::new();
    let mut settle_intents = Vec::new();
    // Decide a pseudo-random sequence of trade events from the byte stream.
    for (index, byte) in data.iter().enumerate().take(16) {
        match byte % 4 {
            0 => events.push(event(index, tenant, "trade.lock", &trade_id)),
            1 => events.push(event(index, tenant, "trade.settle", &trade_id)),
            2 => events.push(event(index, tenant, "trade.unlock", &trade_id)),
            _ => {
                // A balance.transfer value-pair event.
                events.push(event(index, tenant, "balance.transfer", &trade_id));
            }
        }
    }
    if data.len() > 2 && data[2].is_multiple_of(3) {
        let amount = format!("{}", data.get(3).copied().unwrap_or(0) % 100);
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert(String::from("trade_id"), serde_json::json!(trade_id));
        inputs.insert(
            String::from("value_resource"),
            serde_json::json!("wallet:gold"),
        );
        inputs.insert(String::from("value_amount"), serde_json::json!(amount));
        inputs.insert(String::from("value_to_subject"), serde_json::json!("alice"));
        settle_intents.push(Intent::new(
            TenantId(String::from(tenant)),
            IntentId::new(String::from("int_settle_value")).ok()?,
            Operation::from_static("trade.settle"),
            SubjectId(String::from("account:example:player")),
            ResourceId(String::from("asset:sword")),
            None,
            1,
            inputs,
            None,
            now(),
            None,
            Nonce::from_bytes(vec![0]).ok()?,
        ));
    }
    let commit = Commit::new(
        CommitScope::tenant(TenantId(String::from(tenant))),
        CommitId::new(format!(
            "cmt_{id:08}",
            id = data.get(4).copied().unwrap_or(1)
        ))
        .ok()?,
        None,
        1,
        events.len() as u64,
        hash_bytes(b"event-root"),
        hash_bytes(b"previous-root"),
        hash_bytes(b"next-root"),
        now(),
        SubjectId(String::from("service:statechronicle.example.net")),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).ok()?,
    );
    let signed = Signed::new(
        commit,
        SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:z6Mk...#fuzz")).ok()?,
            sig: Signature::from_bytes([0u8; 64]),
        },
    );
    Some(IngestBatch {
        events,
        settle_intents,
        commit: signed,
    })
}
