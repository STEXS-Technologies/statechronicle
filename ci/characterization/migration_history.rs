//! Run only against the pinned original release to create a new immutable corpus.
#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]
mod common;
#[path = "common/domains.rs"]
mod domains;
use common::Harness;
use domains::{Configuration, ConfigurationCommand};
use serde_json::json;
use statechronicle::accumulator::sparse_merkle::StateAccumulator;
use statechronicle::commit::{roots::state_root_updates, sign::sign_commit};
use statechronicle::core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle::{CommitId, ContentDigest, StateProfile, StateType};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let destination = std::env::args().nth(1).expect("destination path");
    let h = Harness::with_registry(domains::registry().unwrap());
    let mut events = Vec::new();
    let mut commits = Vec::new();
    let mut intents = Vec::new();
    let mut tree = StateAccumulator::empty();
    let mut parent = None;
    let mut previous = ContentDigest::new(*tree.root().as_bytes());
    for step in 0_u64..4 {
        let command = if step == 0 {
            ConfigurationCommand::Create { enabled: true }
        } else {
            ConfigurationCommand::SetEnabled { enabled: step % 2 == 0 }
        };
        let raw = json!({
            "schema": "statechronicle.intent.v0", "tenant_id": "acme.game.alpha",
            "intent_id": format!("int_original_configuration_{step}"),
            "operation": Configuration.operation(&command), "actor": "operator:alice",
            "resource_id": "config:version1", "state_type": "custom", "expected_version": step,
            "inputs": { "profile": Configuration::DOMAIN_ID, "schema_version": 1,
                "command": canonicalize(&command).unwrap() },
            "created_at": "2026-07-14T00:00:00Z", "nonce": "b64u:AA"
        });
        let accepted = h.accept(&raw, Some(h.authority()));
        let event = h.run(&accepted, StateType::Custom).await;
        let (mut commit, _) = h.commit_events(std::slice::from_ref(&event));
        tree.insert_batch(&state_root_updates(std::slice::from_ref(&event)).unwrap()).unwrap();
        commit.body.commit_id = CommitId::new(format!("cmt_original_configuration_{step}")).unwrap();
        commit.body.sequence = step + 1;
        commit.body.parent_commit_id = parent;
        commit.body.previous_state_root = previous;
        commit.body.next_state_root = ContentDigest::new(*tree.root().as_bytes());
        parent = Some(commit.body.commit_id.clone());
        previous = commit.body.next_state_root.clone();
        commit = sign_commit(&commit.body, &common::fixed_key(), h.key_id()).unwrap();
        events.push(event); commits.push(commit); intents.push(accepted.intent);
    }
    let output = json!({
        "producer_commit": "c418e98b91720188158c531cd5c2c4b8ab71cbcf",
        "events_bcs": events.iter().map(|e| canonicalize(e).unwrap()).collect::<Vec<_>>(),
        "commits_bcs": commits.iter().map(|c| canonicalize(c).unwrap()).collect::<Vec<_>>(),
        "intents_bcs": intents.iter().map(|i| canonicalize(i).unwrap()).collect::<Vec<_>>(),
        "tip_digest": canonicalize_and_digest(&commits.last().unwrap().body).unwrap(),
        "final_root": previous, "intents": intents, "events": events, "commits": commits,
    });
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(destination).unwrap();
    serde_json::to_writer_pretty(file, &output).unwrap();
}
