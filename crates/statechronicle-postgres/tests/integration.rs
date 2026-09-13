//! Live PostgreSQL adapter checks. Set `STATECHRONICLE_POSTGRES_URL` to run;
//! local builds without PostgreSQL skip the test deliberately.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use statechronicle_commit::persist::SignedCommitVerifier;
use statechronicle_commit::roots::event_root;
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::hash_bytes;
use statechronicle_core::limits::{MAX_EVENTS_PER_COMMIT, MAX_OUTBOX_PAYLOAD_BYTES};
use statechronicle_core::signature::Signature;
use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::IntentId;
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::{
    Intent, KeyId, Nonce, Operation, SignatureAlg, SignatureBlock,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::{ResourceState, UniqueAssetState};
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_ports::commit_store::CommitStore;
use statechronicle_ports::ledger_store::{IdempotencyClaim, LedgerStore, OutboxRecord};
use statechronicle_ports::outbox::{
    ConsumerDedupStore, ConsumerDeliveryClaim, OutboxPayload, OutboxPublisher, OutboxStore,
    PoisonPolicy, dispatch_once_with_policy,
};
use statechronicle_postgres::PostgresLedgerStore;

struct NoopPublisher;

struct AcceptVerifier;

impl SignedCommitVerifier for AcceptVerifier {
    fn verify(&self, _commit: &Signed<Commit>) -> Result<(), String> {
        Ok(())
    }
}

struct RejectVerifier;

impl SignedCommitVerifier for RejectVerifier {
    fn verify(&self, _commit: &Signed<Commit>) -> Result<(), String> {
        Err(String::from("key revoked"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_schema_installations_are_serialized() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let url = url.to_string_lossy().into_owned();
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let url = url.clone();
        tasks.push(tokio::spawn(async move {
            let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
            let connection_task = tokio::spawn(connection);
            let result = client
                .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
                .await;
            connection_task.abort();
            result
        }));
    }
    for task in tasks {
        task.await
            .expect("schema installer task must not panic")
            .expect("concurrent schema installation must succeed");
    }
    PostgresLedgerStore::new_verified(url)
        .await
        .expect("verified startup constructor must accept the initialized schema");
}

#[async_trait::async_trait]
impl OutboxPublisher for NoopPublisher {
    async fn publish(&self, _delivery_key: &str, _payload: &OutboxPayload) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn live_postgres_reservation_rolls_back() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let (client, connection) =
        tokio_postgres::connect(&url.to_string_lossy(), tokio_postgres::NoTls)
            .await
            .expect("PostgreSQL service must be reachable when integration test is enabled");
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("test PostgreSQL connection ended: {error}");
        }
    });
    client
        .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
        .await
        .expect("baseline PostgreSQL schema must apply");
    let required_constraints: i64 = client
        .query_one(
            "SELECT count(*) FROM pg_constraint
             WHERE conname IN ('sc_heads_commit_fk','sc_idempotency_commit_fk')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(required_constraints, 2);

    let store = PostgresLedgerStore::new(url.to_string_lossy().into_owned());
    let tenant = TenantId(String::from("integration.game"));
    let intent = Intent::new(
        tenant.clone(),
        IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
        Operation::from_static("asset.transfer"),
        SubjectId(String::from("account:alice")),
        ResourceId(String::from("asset:sword")),
        Some(StateType::UniqueAsset),
        1,
        std::collections::BTreeMap::new(),
        None,
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        None,
        Nonce::from_bytes(vec![1]).unwrap(),
    );
    let digest = canonicalize_and_digest(&intent).unwrap();
    let mut transaction = store.begin(&tenant).await.unwrap();
    assert!(matches!(
        transaction
            .claim_idempotency(&tenant, &intent, &digest)
            .await
            .unwrap(),
        IdempotencyClaim::NewReservation { .. }
    ));
    transaction.rollback().await.unwrap();

    let mut retry = store.begin(&tenant).await.unwrap();
    assert!(matches!(
        retry
            .claim_idempotency(&tenant, &intent, &digest)
            .await
            .unwrap(),
        IdempotencyClaim::NewReservation { .. }
    ));
    retry.rollback().await.unwrap();
}

#[tokio::test]
async fn live_postgres_expired_reservation_is_taken_over_once() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let store = PostgresLedgerStore::new(url.to_string_lossy().into_owned());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tenant = TenantId(format!("lease.{suffix}"));
    let (mut intent, _, _) = multi_fixture(&tenant, &suffix);
    intent.intent_id = IntentId::new(format!("int_01JZ8W1LEASE{suffix}"))
        .expect("generated intent id must be valid");
    let digest = canonicalize_and_digest(&intent).unwrap();
    let (client, connection) =
        tokio_postgres::connect(&url.to_string_lossy(), tokio_postgres::NoTls)
            .await
            .unwrap();
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("test PostgreSQL connection ended: {error}");
        }
    });
    client
        .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO sc_idempotency
             (tenant_id,intent_id,payload_digest,status,attempt_id,lease_expires_at,intent_payload)
             VALUES ($1,$2,$3,'in_progress','crashed',to_timestamp(0),$4)",
            &[
                &tenant.0,
                &intent.intent_id.0,
                &digest.as_bytes().as_slice(),
                &bcs::to_bytes(&intent).unwrap(),
            ],
        )
        .await
        .unwrap();
    let mut tx = store.begin(&tenant).await.unwrap();
    let claim = tx
        .claim_idempotency(&tenant, &intent, &digest)
        .await
        .unwrap();
    let attempt = match claim {
        IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
        IdempotencyClaim::Committed { .. }
        | IdempotencyClaim::InProgress { .. }
        | IdempotencyClaim::ConflictDifferentPayload => {
            panic!("expired reservation was not taken over")
        }
    };
    assert_ne!(attempt, "crashed");
    tx.rollback().await.unwrap();
    let mut second = store.begin(&tenant).await.unwrap();
    // The takeover was part of the rolled-back transaction, so rollback must
    // restore the expired reservation and permit a clean retry rather than
    // leaving a phantom in-progress lease behind.
    assert!(matches!(
        second
            .claim_idempotency(&tenant, &intent, &digest)
            .await
            .unwrap(),
        IdempotencyClaim::NewReservation { .. }
    ));
    second.rollback().await.unwrap();
    client
        .execute(
            "DELETE FROM sc_idempotency WHERE tenant_id=$1 AND intent_id=$2",
            &[&tenant.0, &intent.intent_id.0],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn live_postgres_persists_commit_projection_and_outbox_atomically() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let url = url.to_string_lossy().into_owned();
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("connection ended: {error}");
        }
    });
    client
        .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
        .await
        .unwrap();

    let store = PostgresLedgerStore::new(url);
    let tenant = TenantId(String::from("integration.atomic"));
    assert!(
        store
            .claim("integration-worker", 1_025, 2_000_000_000)
            .await
            .is_err()
    );
    let request = Intent::new(
        tenant.clone(),
        IntentId::new(String::from("int_01JZ8W1DURABLEPOSTGRES01")).unwrap(),
        Operation::from_static("asset.transfer"),
        SubjectId(String::from("account:alice")),
        ResourceId(String::from("asset:sword")),
        Some(StateType::UniqueAsset),
        1,
        std::collections::BTreeMap::new(),
        None,
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        None,
        Nonce::from_bytes(vec![1]).unwrap(),
    );
    let state = ResourceState::UniqueAsset(UniqueAssetState {
        owner: SubjectId(String::from("account:alice")),
        status: Status::from_static("active"),
        trade_id: None,
    });
    let state_hash = canonicalize_and_digest(&state).unwrap();
    let event = Event::new(
        tenant.clone(),
        EventId::new(String::from("evt_01JZ8W1DURABLEPOSTGRES01")).unwrap(),
        request.intent_id.clone(),
        request.operation.clone(),
        request.resource_id.clone(),
        SubjectId(String::from("account:alice")),
        StateCommitment {
            version: 0,
            state_hash: state_hash.clone(),
            state: state.clone(),
        },
        StateCommitment {
            version: 1,
            state_hash: state_hash.clone(),
            state: state.clone(),
        },
        None,
        SubjectId(String::from("service:ledger")),
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    let commit_id = CommitId::new(String::from("cmt_01JZ8W1DURABLEPOSTGRES01")).unwrap();
    let body = Commit::new(
        CommitScope::tenant(tenant.clone()),
        commit_id.clone(),
        None,
        1,
        1,
        event_root(std::slice::from_ref(&event)).unwrap(),
        hash_bytes(b"genesis"),
        hash_bytes(b"next"),
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        SubjectId(String::from("service:ledger")),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    );
    let signed = Signed::new(
        body,
        SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:z6Mk...#ledger")).unwrap(),
            sig: Signature::from_bytes([0u8; 64]),
        },
    );
    let projection = StateProjection {
        tenant_id: tenant.clone(),
        resource_id: request.resource_id.clone(),
        state_type: StateType::UniqueAsset,
        version: 1,
        last_event_id: event.event_id.clone(),
        last_commit_id: commit_id.clone(),
        state_hash,
        state,
    };
    let outbox = OutboxRecord {
        delivery_key: String::from("commit:integration.atomic:cmt_01JZ8W1DURABLEPOSTGRES01"),
        tenant: tenant.clone(),
        commit_id: commit_id.clone(),
        payload_digest: hash_bytes(b"payload"),
        payload: b"payload".to_vec(),
    };
    let mut outbox_limit_tx = store.begin(&tenant).await.unwrap();
    let oversized_payload = vec![0u8; MAX_OUTBOX_PAYLOAD_BYTES + 1];
    let oversized_outbox = OutboxRecord {
        delivery_key: String::from("commit:integration.atomic:oversized"),
        tenant: tenant.clone(),
        commit_id: commit_id.clone(),
        payload_digest: hash_bytes(&oversized_payload),
        payload: oversized_payload,
    };
    assert!(
        outbox_limit_tx
            .enqueue_outbox(&oversized_outbox)
            .await
            .is_err()
    );
    outbox_limit_tx.rollback().await.unwrap();
    let mut limit_tx = store.begin(&tenant).await.unwrap();
    let oversized = vec![event.clone(); MAX_EVENTS_PER_COMMIT + 1];
    assert!(limit_tx.append_events(&oversized).await.is_err());
    limit_tx.rollback().await.unwrap();
    let digest = canonicalize_and_digest(&request).unwrap();
    let mut tx = store.begin(&tenant).await.unwrap();
    let attempt = match tx
        .claim_idempotency(&tenant, &request, &digest)
        .await
        .unwrap()
    {
        IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
        IdempotencyClaim::Committed { .. }
        | IdempotencyClaim::InProgress { .. }
        | IdempotencyClaim::ConflictDifferentPayload => panic!("unexpected idempotency claim"),
    };
    let mut wrong_actor_event = event.clone();
    wrong_actor_event.actor = SubjectId(String::from("account:mallory"));
    assert!(matches!(
        tx.append_events(&[wrong_actor_event]).await,
        Err(statechronicle_ports::ledger_store::LedgerStoreError::Invariant(message))
            if message.contains("claimed intent")
    ));
    let mut wrong_operation_event = event.clone();
    wrong_operation_event.operation = Operation::from_static("asset.burn");
    assert!(matches!(
        tx.append_events(&[wrong_operation_event]).await,
        Err(statechronicle_ports::ledger_store::LedgerStoreError::Invariant(message))
            if message.contains("claimed intent")
    ));
    tx.append_events(std::slice::from_ref(&event))
        .await
        .unwrap();
    let mut invalid_commit = signed.clone();
    invalid_commit.body.event_count = 2;
    assert!(tx.append_commit(&invalid_commit).await.is_err());
    // A rejected append preserves staged events; use the correctly bound
    // commit for the actual atomic write.
    tx.append_commit(&signed).await.unwrap();
    tx.upsert_projection(&projection).await.unwrap();
    tx.enqueue_outbox(&outbox).await.unwrap();
    let unrelated_commit = CommitId::new(String::from("cmt_unrelated")).unwrap();
    assert!(
        tx.finalize_idempotency(&tenant, &request.intent_id, &attempt, &unrelated_commit,)
            .await
            .is_err()
    );
    tx.finalize_idempotency(&tenant, &request.intent_id, &attempt, &commit_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        store
            .commit_by_id(&tenant, &commit_id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .canonical_head(&tenant)
            .await
            .unwrap()
            .unwrap()
            .commit_id,
        commit_id
    );
    store.verify_integrity(&tenant).await.unwrap();
    store
        .verify_integrity_with_verifier(&tenant, &AcceptVerifier)
        .await
        .unwrap();
    assert!(
        store
            .verify_integrity_with_verifier(&tenant, &RejectVerifier)
            .await
            .is_err()
    );
    assert!(
        store
            .verify_all_integrity_with_verifier(&AcceptVerifier)
            .await
            .unwrap()
            .contains(&tenant)
    );
    let canonical = store.canonical_events(&tenant).await.unwrap();
    assert_eq!(canonical.len(), 1);
    assert_eq!(canonical.first().unwrap().0.event_id, event.event_id);
    assert!(
        store
            .verify_all_integrity()
            .await
            .unwrap()
            .contains(&tenant)
    );
    assert!(
        client
            .execute(
                "UPDATE sc_projections SET version=0 WHERE tenant_id=$1 AND resource_id=$2",
                &[&tenant.0, &projection.resource_id.0],
            )
            .await
            .is_err()
    );
    client
        .execute(
            "DELETE FROM sc_projections WHERE tenant_id=$1",
            &[&tenant.0],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .rebuild_projections_from_canonical(&tenant, true)
            .await
            .unwrap(),
        1
    );
    let claimed = store
        .claim("integration-worker", 10, 2_000_000_000)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    let Some((claimed_key, _claimed_payload)) = claimed.into_iter().next() else {
        panic!("expected one claimed outbox row");
    };
    assert_eq!(claimed_key, outbox.delivery_key);
    assert!(
        store
            .claim("other-integration-worker", 10, 4_000_000_000)
            .await
            .unwrap()
            .is_empty()
    );
    client
        .execute(
            "UPDATE sc_outbox SET lease_until=now()-interval '1 second' WHERE delivery_key=$1",
            &[&claimed_key],
        )
        .await
        .unwrap();
    let reclaimed = store
        .claim("other-integration-worker", 10, 4_000_000_000)
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert!(
        store
            .mark_delivered_by(&claimed_key, "integration-worker")
            .await
            .is_err()
    );
    store
        .mark_delivered_by(&claimed_key, "other-integration-worker")
        .await
        .unwrap();
    // Completion is idempotent even when a client retries after the broker
    // acknowledged the first completion response.
    store
        .mark_delivered_by(&claimed_key, "other-integration-worker")
        .await
        .unwrap();
    assert_eq!(store.pending_count(Some(&tenant)).await.unwrap(), 0);

    let delivery_key = "consumer:integration.atomic:cmt_01JZ8W1DURABLEPOSTGRES01";
    let consumer_attempt = match store
        .claim_delivery(delivery_key, 2_000_000_000)
        .await
        .unwrap()
    {
        ConsumerDeliveryClaim::New { attempt_id } => attempt_id,
        ConsumerDeliveryClaim::AlreadyApplied | ConsumerDeliveryClaim::InProgress { .. } => {
            panic!("expected a new consumer delivery claim")
        }
    };
    store
        .mark_delivery_applied(delivery_key, &consumer_attempt)
        .await
        .unwrap();
    assert!(matches!(
        store
            .claim_delivery(delivery_key, 2_000_000_000)
            .await
            .unwrap(),
        ConsumerDeliveryClaim::AlreadyApplied
    ));
    client
        .execute(
            "INSERT INTO sc_outbox (delivery_key,tenant_id,commit_id,payload_digest,payload)
             VALUES ($1,$2,$3,$4,$5)",
            &[
                &"poison:integration.atomic",
                &tenant.0,
                &commit_id.0,
                &vec![1u8; 32],
                &b"poison".to_vec(),
            ],
        )
        .await
        .unwrap();
    let report = dispatch_once_with_policy(
        &store,
        &NoopPublisher,
        "poison-worker",
        10,
        2_000_000_000,
        PoisonPolicy::Quarantine,
    )
    .await
    .unwrap();
    assert_eq!(report.quarantined, 1);
    assert!(
        client
            .execute(
                "UPDATE sc_commits SET payload=payload WHERE tenant_id=$1 AND commit_id=$2",
                &[&tenant.0, &commit_id.0],
            )
            .await
            .is_err()
    );
    assert!(
        client
            .execute(
                "UPDATE sc_idempotency SET status='in_progress',commit_id=NULL
             WHERE tenant_id=$1 AND intent_id=$2",
                &[&tenant.0, &request.intent_id.0],
            )
            .await
            .is_err()
    );
    assert!(
        client
            .execute(
                "UPDATE sc_idempotency SET payload_digest=$3
             WHERE tenant_id=$1 AND intent_id=$2",
                &[
                    &tenant.0,
                    &request.intent_id.0,
                    &hash_bytes(b"different").as_bytes().to_vec(),
                ],
            )
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn live_postgres_concurrent_idempotency_has_one_winner() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let url = url.to_string_lossy().into_owned();
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("connection ended: {error}");
        }
    });
    client
        .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
        .await
        .unwrap();
    let tenant = TenantId(String::from("integration.concurrent"));
    let intent = Intent::new(
        tenant.clone(),
        IntentId::new(String::from("int_01JZ8W1CONCURRENTPOSTGRES1")).unwrap(),
        Operation::from_static("asset.transfer"),
        SubjectId(String::from("account:alice")),
        ResourceId(String::from("asset:sword")),
        Some(StateType::UniqueAsset),
        1,
        std::collections::BTreeMap::new(),
        None,
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        None,
        Nonce::from_bytes(vec![1]).unwrap(),
    );
    let digest = canonicalize_and_digest(&intent).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let url = url.clone();
        let tenant = tenant.clone();
        let intent = intent.clone();
        let digest = digest.clone();
        tasks.push(tokio::spawn(async move {
            let store = PostgresLedgerStore::new(url);
            let mut tx = store.begin(&tenant).await.unwrap();
            let claim = tx
                .claim_idempotency(&tenant, &intent, &digest)
                .await
                .unwrap();
            (claim, tx)
        }));
    }
    let mut winners = 0usize;
    let mut transactions = Vec::new();
    for task in tasks {
        let (claim, tx) = task.await.unwrap();
        if matches!(claim, IdempotencyClaim::NewReservation { .. }) {
            winners += 1;
        }
        transactions.push(tx);
    }
    assert_eq!(winners, 1);
    for tx in transactions {
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn live_postgres_multi_tenant_commit_is_one_transaction() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let url = url.to_string_lossy().into_owned();
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("connection ended: {error}");
        }
    });
    client
        .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
        .await
        .unwrap();

    let tenant_a = TenantId(String::from("integration.multi.a"));
    let tenant_b = TenantId(String::from("integration.multi.b"));
    let (intent_a, event_a, commit_a) = multi_fixture(&tenant_a, "A");
    let (intent_b, event_b, commit_b) = multi_fixture(&tenant_b, "B");
    let digest_a = canonicalize_and_digest(&intent_a).unwrap();
    let digest_b = canonicalize_and_digest(&intent_b).unwrap();
    let mut tx = PostgresLedgerStore::new(url)
        .begin_multi(&[tenant_b.clone(), tenant_a.clone()])
        .await
        .unwrap();
    let attempt_a = match tx
        .claim_idempotency(&tenant_a, &intent_a, &digest_a)
        .await
        .unwrap()
    {
        IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
        IdempotencyClaim::Committed { .. }
        | IdempotencyClaim::InProgress { .. }
        | IdempotencyClaim::ConflictDifferentPayload => {
            panic!("tenant A reservation must be new")
        }
    };
    let attempt_b = match tx
        .claim_idempotency(&tenant_b, &intent_b, &digest_b)
        .await
        .unwrap()
    {
        IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
        IdempotencyClaim::Committed { .. }
        | IdempotencyClaim::InProgress { .. }
        | IdempotencyClaim::ConflictDifferentPayload => {
            panic!("tenant B reservation must be new")
        }
    };
    tx.append_events(&[event_a.clone(), event_b.clone()])
        .await
        .unwrap();
    tx.append_commit(&commit_a).await.unwrap();
    tx.append_commit(&commit_b).await.unwrap();
    tx.finalize_idempotency(
        &tenant_a,
        &intent_a.intent_id,
        &attempt_a,
        &commit_a.body.commit_id,
    )
    .await
    .unwrap();
    tx.finalize_idempotency(
        &tenant_b,
        &intent_b.intent_id,
        &attempt_b,
        &commit_b.body.commit_id,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let store = PostgresLedgerStore::new(std::env::var("STATECHRONICLE_POSTGRES_URL").unwrap());
    assert_eq!(
        store
            .canonical_head(&tenant_a)
            .await
            .unwrap()
            .unwrap()
            .commit_id,
        commit_a.body.commit_id
    );
    assert_eq!(
        store
            .canonical_head(&tenant_b)
            .await
            .unwrap()
            .unwrap()
            .commit_id,
        commit_b.body.commit_id
    );
    store.verify_integrity(&tenant_a).await.unwrap();
    store.verify_integrity(&tenant_b).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_postgres_canonical_head_race_has_one_winner() {
    let Some(url) = std::env::var_os("STATECHRONICLE_POSTGRES_URL") else {
        eprintln!("STATECHRONICLE_POSTGRES_URL not set; skipping live PostgreSQL test");
        return;
    };
    let url = url.to_string_lossy().into_owned();
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("connection ended: {error}");
        }
    });
    client
        .batch_execute(include_str!("../../../docs/OPERATIONS/postgres_schema.sql"))
        .await
        .unwrap();

    let tenant = TenantId(String::from("integration.head-race"));
    let (intent_a, event_a, commit_a) = multi_fixture(&tenant, "C");
    let (intent_b, event_b, commit_b) = multi_fixture(&tenant, "D");
    let digest_a = canonicalize_and_digest(&intent_a).unwrap();
    let digest_b = canonicalize_and_digest(&intent_b).unwrap();
    let tasks = [
        (intent_a, event_a, commit_a, digest_a),
        (intent_b, event_b, commit_b, digest_b),
    ]
    .into_iter()
    .map(|(intent, event, commit, digest)| {
        let url = url.clone();
        let tenant = tenant.clone();
        tokio::spawn(async move {
            let store = PostgresLedgerStore::new(url);
            let mut tx = store.begin(&tenant).await.unwrap();
            let attempt = match tx
                .claim_idempotency(&tenant, &intent, &digest)
                .await
                .unwrap()
            {
                IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
                IdempotencyClaim::Committed { .. }
                | IdempotencyClaim::InProgress { .. }
                | IdempotencyClaim::ConflictDifferentPayload => return false,
            };
            tx.append_events(std::slice::from_ref(&event))
                .await
                .unwrap();
            if tx.append_commit(&commit).await.is_err() {
                if tx.rollback().await.is_err() {
                    return false;
                }
                return false;
            }
            tx.finalize_idempotency(&tenant, &intent.intent_id, &attempt, &commit.body.commit_id)
                .await
                .unwrap();
            tx.commit().await.is_ok()
        })
    });
    let mut winners = 0usize;
    for task in tasks {
        if task.await.unwrap() {
            winners += 1;
        }
    }
    assert_eq!(winners, 1);
    let head = PostgresLedgerStore::new(std::env::var("STATECHRONICLE_POSTGRES_URL").unwrap())
        .canonical_head(&tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.sequence, 1);
    PostgresLedgerStore::new(std::env::var("STATECHRONICLE_POSTGRES_URL").unwrap())
        .verify_integrity(&tenant)
        .await
        .unwrap();
}

fn multi_fixture(tenant: &TenantId, suffix: &str) -> (Intent, Event, Signed<Commit>) {
    let intent = Intent::new(
        tenant.clone(),
        IntentId::new(format!("int_01JZ8W1MULTI{suffix}000000000000")).unwrap(),
        Operation::from_static("asset.transfer"),
        SubjectId(format!("account:alice-{suffix}")),
        ResourceId(format!("asset:sword-{suffix}")),
        Some(StateType::UniqueAsset),
        1,
        std::collections::BTreeMap::new(),
        None,
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        None,
        Nonce::from_bytes(vec![1]).unwrap(),
    );
    let state = ResourceState::UniqueAsset(UniqueAssetState {
        owner: SubjectId(format!("account:alice-{suffix}")),
        status: Status::from_static("active"),
        trade_id: None,
    });
    let state_hash = canonicalize_and_digest(&state).unwrap();
    let event = Event::new(
        tenant.clone(),
        EventId::new(format!("evt_01JZ8W1MULTI{suffix}000000000000")).unwrap(),
        intent.intent_id.clone(),
        intent.operation.clone(),
        intent.resource_id.clone(),
        intent.actor.clone(),
        StateCommitment {
            version: 0,
            state_hash: state_hash.clone(),
            state: state.clone(),
        },
        StateCommitment {
            version: 1,
            state_hash,
            state,
        },
        None,
        SubjectId(String::from("service:ledger")),
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    let commit_id = CommitId::new(format!("cmt_01JZ8W1MULTI{suffix}000000000000")).unwrap();
    let body = Commit::new(
        CommitScope::tenant(tenant.clone()),
        commit_id,
        None,
        1,
        1,
        event_root(std::slice::from_ref(&event)).unwrap(),
        hash_bytes(b"genesis"),
        hash_bytes(format!("next-{suffix}").as_bytes()),
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        SubjectId(String::from("service:ledger")),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    );
    let signed = Signed::new(
        body,
        SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:z6Mk...#ledger")).unwrap(),
            sig: Signature::from_bytes([0u8; 64]),
        },
    );
    (intent, event, signed)
}
