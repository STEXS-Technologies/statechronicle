//! Release-mode baseline for the legacy Ed25519 signing path.
#![allow(clippy::unwrap_used)]

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use statechronicle_commit::sign::{sign_commit, verify_commit};
use statechronicle_core::{
    digest::hash_bytes,
    signature::{sign, verify},
};
use statechronicle_domain::{
    commit::{Commit, CommitScope, ProfileId},
    ids::CommitId,
    intent::KeyId,
    subject::SubjectId,
    tenant::TenantId,
};
use std::{hint::black_box, time::Instant};

const SEED: [u8; 32] = [42; 32];
const SAMPLES: usize = 10_000;
const WARMUP: usize = 100;

fn commit() -> Commit {
    let d = hash_bytes(b"statechronicle signature agility baseline");
    Commit::new(
        CommitScope::tenant(TenantId(String::from("acme.game.alpha"))),
        CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
        Some(CommitId::new(String::from("cmt_01JZ8WZ0QH93JK8J19VVD3QXSC")).unwrap()),
        918273,
        180000,
        d.clone(),
        d.clone(),
        d,
        DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
            .unwrap()
            .with_timezone(&Utc),
        SubjectId(String::from("service:statechronicle.example.net")),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    )
}
fn key_id() -> KeyId {
    KeyId::new(String::from("did:key:z6Mk...#statechronicle-commit")).unwrap()
}

fn measure(name: &str, mut operation: impl FnMut()) {
    for _ in 0..WARMUP {
        operation();
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let [p50, p95, p99] = [50, 95, 99].map(|percentile| {
        let rank = samples.len().checked_mul(percentile).unwrap().div_ceil(100);
        *samples.get(rank.saturating_sub(1)).unwrap()
    });
    println!("{name}: p50_ns={p50} p95_ns={p95} p99_ns={p99}");
}

fn main() {
    let key = SigningKey::from_bytes(&SEED);
    let verifying_key = key.verifying_key();
    let message = [0x5a; 1024];
    let signature = sign(&message, &key);
    let body = commit();
    let key_id = key_id();
    let signed = sign_commit(&body, &key, key_id.clone()).unwrap();

    measure("ed25519_sign_1024b", || {
        black_box(sign(black_box(&message), black_box(&key)));
    });
    measure("ed25519_verify_1024b", || {
        verify(
            black_box(&message),
            black_box(&verifying_key),
            black_box(&signature),
        )
        .unwrap();
    });
    measure("commit_sign", || {
        black_box(sign_commit(black_box(&body), black_box(&key), key_id.clone()).unwrap());
    });
    measure("commit_verify", || {
        verify_commit(black_box(&signed), black_box(&verifying_key)).unwrap();
    });
}
