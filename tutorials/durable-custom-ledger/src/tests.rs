use super::*;
use statechronicle::ports::authorization::DenyAllAuthorizer;
use statechronicle::ports::ledger_store::{
    IdempotencyClaim, LedgerLookup, LedgerRecovery, LedgerStore,
};
use statechronicle::proof::error::ProofError;
use std::{collections::BTreeMap, fs};

async fn genesis(
    directory: &tempfile::TempDir,
) -> Result<
    (
        Application,
        statechronicle::intent::validated::ValidatedIntent,
        statechronicle::mutation::SignedMutation,
    ),
    Box<dyn Error>,
> {
    let app = Application::new(FileLedger::open(
        directory.path(),
        tenant(),
        &[None],
        &CommitAuthority,
    )?)?;
    let request = app.request(
        "int_configuration_create",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: true },
    )?;
    let plan = app.plan(&request).await?;
    Ok((app, request, plan))
}
fn raw_records(directory: &tempfile::TempDir) -> Result<usize, Box<dyn Error>> {
    let data: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.path().join("journal.json"))?)?;
    Ok(data["records"].as_array().ok_or("missing records")?.len())
}

#[tokio::test]
async fn complete_demo_runs_with_both_domains_and_temporal_proofs() -> Result<(), Box<dyn Error>> {
    demonstrate().await
}

#[tokio::test]
async fn lost_ack_reopens_actual_file_and_replays_original_effects_without_reduction()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, request, plan) = genesis(&directory).await?;
    assert!(matches!(
        plan.commit(&app.ledger.lose_next_ack(), &app.policy, &CommitAuthority)
            .await,
        Err(DurableError::Unknown { .. })
    ));
    assert_eq!(raw_records(&directory)?, 1);
    let pin = tip(&plan)?;
    drop(app);
    let reopened = Application::new(FileLedger::open(
        directory.path(),
        tenant(),
        &[Some(pin)],
        &CommitAuthority,
    )?)?;
    let receipt = match recover(
        &reopened.ledger,
        &reopened.policy,
        &CommitAuthority,
        &reopened.principal,
        &request.intent,
    )
    .await?
    {
        RecoveryOutcome::Committed(receipt) => receipt,
        other => return Err(format!("unexpected recovery: {other:?}").into()),
    };
    assert_eq!(receipt.commit(), plan.signed_commit());
    assert_eq!(receipt.record().events, plan.prepared().events());
    assert_eq!(receipt.record().outbox.len(), 1);
    let unchanged = fs::read(directory.path().join("journal.json"))?;
    assert!(matches!(
        plan.commit(&reopened.ledger, &reopened.policy, &CommitAuthority)
            .await?,
        DurableOutcome::Replay(_)
    ));
    assert_eq!(fs::read(directory.path().join("journal.json"))?, unchanged);
    let mut changed = request.intent.clone();
    changed.nonce = statechronicle::domain::intent::Nonce::from_bytes(vec![2])?;
    assert!(matches!(
        recover(
            &reopened.ledger,
            &reopened.policy,
            &CommitAuthority,
            &reopened.principal,
            &changed
        )
        .await,
        Err(DurableError::PayloadConflict)
    ));
    assert_eq!(fs::read(directory.path().join("journal.json"))?, unchanged);
    Ok(())
}

#[tokio::test]
async fn failure_before_commit_rolls_back_every_effect_and_allows_exact_plan_retry()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, request, plan) = genesis(&directory).await?;
    assert!(matches!(
        plan.commit(
            &app.ledger.fail_before_commit(),
            &app.policy,
            &CommitAuthority
        )
        .await,
        Err(DurableError::Unavailable(_))
    ));
    assert!(!directory.path().join("journal.json").exists());
    assert!(app.ledger.records()?.is_empty());
    assert!(matches!(
        app.ledger
            .lookup_mutation(&tenant(), &request.intent.intent_id)
            .await?,
        LedgerLookup::NotFound
    ));
    assert!(matches!(
        plan.commit(&app.ledger, &app.policy, &CommitAuthority)
            .await?,
        DurableOutcome::Committed(_)
    ));
    assert_eq!(raw_records(&directory)?, 1);
    Ok(())
}

#[tokio::test]
async fn io_failure_is_sticky_uncertainty_until_exclusive_reopen() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, request, plan) = genesis(&directory).await?;
    fs::create_dir(directory.path().join("journal.next"))?;
    assert!(matches!(
        plan.commit(&app.ledger, &app.policy, &CommitAuthority)
            .await,
        Err(DurableError::Unknown { .. })
    ));
    assert!(
        app.ledger
            .lookup_mutation(&tenant(), &request.intent.intent_id)
            .await
            .is_err()
    );
    assert!(app.ledger.head_and_updates().is_err());
    assert!(!directory.path().join("journal.json").exists());
    drop(app);
    // This test owns the directory and has proved all old writer handles closed.
    fs::remove_dir(directory.path().join("journal.next"))?;
    let reopened = FileLedger::open(
        directory.path(),
        tenant(),
        &[None, Some(tip(&plan)?)],
        &CommitAuthority,
    )?;
    assert!(matches!(
        reopened
            .lookup_mutation(&tenant(), &request.intent.intent_id)
            .await?,
        LedgerLookup::NotFound
    ));
    // Generic NotFound alone never authorizes a new identity or blind retry.
    Ok(())
}

#[tokio::test]
async fn commit_reauthorizes_and_key_revocation_after_preparation_refuses_writes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, request, plan) = genesis(&directory).await?;
    assert!(matches!(
        plan.commit(&app.ledger, &DenyAllAuthorizer, &CommitAuthority)
            .await,
        Err(DurableError::Authorization(_))
    ));
    app.keys.revoke(
        &request
            .signature
            .as_ref()
            .ok_or("missing actor signature")?
            .key_id,
    )?;
    assert!(matches!(
        plan.commit(&app.ledger, &app.policy, &CommitAuthority)
            .await,
        Err(DurableError::Authorization(_))
    ));
    assert!(!directory.path().join("journal.json").exists());
    assert!(app.ledger.records()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn same_resource_contenders_cannot_both_commit_and_loser_stays_unknown()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, _, first) = genesis(&directory).await?;
    let other_request = app.request(
        "int_other_configuration_create",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: false },
    )?;
    let second = app.plan(&other_request).await?;
    let (left, right) = tokio::join!(
        first.commit(&app.ledger, &app.policy, &CommitAuthority),
        second.commit(&app.ledger, &app.policy, &CommitAuthority)
    );
    assert!(matches!(left, Ok(DurableOutcome::Committed(_))));
    assert!(matches!(right, Err(DurableError::Unknown { .. })));
    assert_eq!(raw_records(&directory)?, 1);
    assert!(matches!(
        app.ledger
            .lookup_mutation(&tenant(), &other_request.intent.intent_id)
            .await?,
        LedgerLookup::NotFound
    ));
    Ok(())
}

#[tokio::test]
async fn exclusive_writer_scope_and_pending_claims_fail_closed() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, request, plan) = genesis(&directory).await?;
    assert!(FileLedger::open(directory.path(), tenant(), &[None], &CommitAuthority).is_err());
    assert!(app.ledger.begin_multi(&[tenant()]).await.is_err());
    assert!(
        app.ledger
            .begin(&statechronicle::TenantId(String::from("other.tenant")))
            .await
            .is_err()
    );
    let mut owner = app.ledger.begin(&tenant()).await?;
    assert!(matches!(
        owner
            .claim_idempotency(&tenant(), &request.intent, plan.prepared().payload_digest())
            .await?,
        IdempotencyClaim::NewReservation { .. }
    ));
    let mut contender = app.ledger.begin(&tenant()).await?;
    assert!(matches!(
        contender
            .claim_idempotency(&tenant(), &request.intent, plan.prepared().payload_digest())
            .await?,
        IdempotencyClaim::InProgress { .. }
    ));
    contender.rollback().await?;
    owner.rollback().await?;
    assert!(matches!(
        app.ledger
            .lookup_mutation(&tenant(), &request.intent.intent_id)
            .await?,
        LedgerLookup::NotFound
    ));
    assert!(!directory.path().join("journal.json").exists());
    Ok(())
}

#[tokio::test]
async fn later_heads_retain_original_records_and_native_replay_matches_both_models()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, request, initial) = genesis(&directory).await?;
    initial
        .commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    let original = app
        .ledger
        .records()?
        .first()
        .ok_or("missing original")?
        .clone();
    let old_proof = app.ledger.state_proof(
        &ResourceId(String::from("configuration:main")),
        &initial.signed_commit().body.commit_id,
    )?;
    let update = app.request(
        "int_configuration_update",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::SetEnabled { enabled: false },
    )?;
    app.plan(&update)
        .await?
        .commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    let equipment = app.request(
        "int_equipment_install",
        "equipment",
        &*EQUIPMENT,
        &EquipmentCommand::Install,
    )?;
    let equipment_plan = app.plan(&equipment).await?;
    equipment_plan
        .commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    let pin = tip(&equipment_plan)?;
    drop(app);
    let app = Application::new(FileLedger::open(
        directory.path(),
        tenant(),
        &[Some(pin)],
        &CommitAuthority,
    )?)?;
    let recovered = recover(
        &app.ledger,
        &app.policy,
        &CommitAuthority,
        &app.principal,
        &request.intent,
    )
    .await?;
    match recovered {
        RecoveryOutcome::Committed(receipt) => assert_eq!(receipt.record(), &original),
        other => return Err(format!("unexpected recovery: {other:?}").into()),
    }
    let mut rebuilt = BTreeMap::new();
    for record in app.ledger.records()? {
        for event in &record.events {
            let previous = rebuilt.get(&event.resource_id);
            if event.resource_id.0 == "configuration:main" {
                CONFIGURATION.verify_transition(previous, &record.intent, event)?;
            } else {
                EQUIPMENT.verify_transition(previous, &record.intent, event)?;
            }
        }
        for projection in record.projections {
            rebuilt.insert(projection.resource_id.clone(), projection);
        }
    }
    let row = rebuilt
        .get(&ResourceId(String::from("configuration:main")))
        .ok_or("missing replay row")?;
    let native = CONFIGURATION.decode_state(&row.state)?;
    assert_eq!(native.revision, 2);
    assert!(!native.enabled);
    assert_eq!(app.ledger.projection(&row.resource_id)?, Some(row.clone()));
    let verifier = ClaimVerifier::new(Box::new(app.ledger.clone()));
    let expected = ExpectedClaim::state(tenant(), ResourceId(String::from("configuration:main")));
    assert!(matches!(
        verifier
            .verify_current(
                ClaimProof::State(&old_proof),
                &expected,
                &trusted_commit_key()
            )
            .await,
        Err(ProofError::NonCanonicalCommit { .. })
    ));
    verifier
        .verify_historical(
            ClaimProof::State(&old_proof),
            &expected,
            &initial.signed_commit().body.commit_id,
            &trusted_commit_key(),
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn corrupt_missing_reordered_or_unpinned_journal_is_refused_before_use()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, _, initial) = genesis(&directory).await?;
    initial
        .commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    let update = app.request(
        "int_configuration_update",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::SetEnabled { enabled: false },
    )?;
    let plan = app.plan(&update).await?;
    plan.commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    let pin = tip(&plan)?;
    drop(app);
    let file = directory.path().join("journal.json");
    let pristine = fs::read(&file)?;
    assert!(FileLedger::open(directory.path(), tenant(), &[None], &CommitAuthority).is_err());
    assert!(
        FileLedger::open(
            directory.path(),
            tenant(),
            &[Some(statechronicle::core::digest::hash_bytes(b"wrong tip"))],
            &CommitAuthority
        )
        .is_err()
    );
    for change in [
        "format",
        "missing_tip",
        "reorder",
        "missing_events",
        "outbox",
        "projection",
        "truncated",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&pristine)?;
        match change {
            "format" => value["format"] = 999.into(),
            "missing_tip" => {
                value["records"].as_array_mut().ok_or("records")?.pop();
            }
            "reorder" => value["records"].as_array_mut().ok_or("records")?.reverse(),
            "missing_events" => value["records"][0]["events"] = serde_json::json!([]),
            "outbox" => value["records"][0]["outbox"][0]["payload"] = serde_json::json!([99]),
            "projection" => value["records"][0]["projections"][0]["version"] = 99.into(),
            "truncated" => {}
            _ => return Err("unknown corruption case".into()),
        }
        let changed = if change == "truncated" {
            b"{".to_vec()
        } else {
            serde_json::to_vec(&value)?
        };
        fs::write(&file, changed)?;
        assert!(
            FileLedger::open(
                directory.path(),
                tenant(),
                &[Some(pin.clone())],
                &CommitAuthority
            )
            .is_err(),
            "{change}"
        );
    }
    fs::write(&file, pristine)?;
    let reopened = FileLedger::open(directory.path(), tenant(), &[Some(pin)], &CommitAuthority)?;
    assert_eq!(reopened.records()?.len(), 2);
    Ok(())
}

// Hooks exist only in the unit-test binary. Environment is set on the child,
// never on the parent process or the production demonstration.
pub(crate) fn crash_selected(path: &std::path::Path, boundary: &str) -> bool {
    std::env::var_os("STATECHRONICLE_CRASH_DIRECTORY")
        .is_some_and(|directory| std::path::Path::new(&directory) == path)
        && std::env::var("STATECHRONICLE_CRASH_BOUNDARY").as_deref() == Ok(boundary)
}
pub(crate) fn crash_boundary(path: &std::path::Path, boundary: &str) {
    if crash_selected(path, boundary) {
        fs::write(path.join("crash.ready.next"), boundary).expect("write crash boundary");
        fs::rename(path.join("crash.ready.next"), path.join("crash.ready"))
            .expect("publish crash boundary");
        // A watchdog bounds accidental direct invocation. Success requires the
        // parent to kill us here, before any normal return or destructor.
        std::thread::sleep(std::time::Duration::from_secs(30));
        panic!("parent did not terminate crash worker");
    }
}

#[tokio::test]
async fn crash_worker() -> Result<(), Box<dyn Error>> {
    let Some(directory) = std::env::var_os("STATECHRONICLE_CRASH_DIRECTORY") else {
        return Ok(());
    };
    let path = std::path::Path::new(&directory);
    let (old, pending): (ContentDigest, ContentDigest) =
        serde_json::from_slice(&fs::read(path.join("crash.pins"))?)?;
    let app = Application::new(FileLedger::open(
        path,
        tenant(),
        &[Some(old)],
        &CommitAuthority,
    )?)?;
    let request = app.request(
        "int_configuration_update",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::SetEnabled { enabled: false },
    )?;
    let plan = app.plan(&request).await?;
    assert_eq!(
        tip(&plan)?,
        pending,
        "child must use the original signed plan"
    );
    plan.commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    Err("worker returned without reaching crash boundary".into())
}

struct CrashChild(std::process::Child);
impl Drop for CrashChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn killed_writer_reopens_at_each_file_boundary_with_exactly_once_effects()
-> Result<(), Box<dyn Error>> {
    use std::os::unix::process::ExitStatusExt;
    for boundary in [
        "before_write",
        "partial_write",
        "after_write",
        "after_file_sync",
        "after_rename",
        "after_directory_sync",
    ] {
        let directory = tempfile::tempdir()?;
        let (app, _, initial) = genesis(&directory).await?;
        initial
            .commit(&app.ledger, &app.policy, &CommitAuthority)
            .await?;
        let original = fs::read(directory.path().join("journal.json"))?;
        let old = tip(&initial)?;
        let request = app.request(
            "int_configuration_update",
            "configuration",
            &*CONFIGURATION,
            &ConfigurationCommand::SetEnabled { enabled: false },
        )?;
        let plan = app.plan(&request).await?;
        let pending = tip(&plan)?;
        fs::write(
            directory.path().join("crash.pins"),
            serde_json::to_vec(&(old.clone(), pending.clone()))?,
        )?;
        drop(app);
        let log = fs::File::create(directory.path().join("crash.log"))?;
        let mut child = CrashChild(
            std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", "tests::crash_worker", "--nocapture"])
                .env("STATECHRONICLE_CRASH_DIRECTORY", directory.path())
                .env("STATECHRONICLE_CRASH_BOUNDARY", boundary)
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !directory.path().join("crash.ready").exists() {
            if let Some(status) = child.0.try_wait()? {
                return Err(format!(
                    "worker exited at {boundary}: {status}: {}",
                    fs::read_to_string(directory.path().join("crash.log"))?
                )
                .into());
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("worker did not reach {boundary}").into());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("crash.ready"))?,
            boundary
        );
        // An independent opener must refuse while the paused process owns the lock.
        assert!(
            FileLedger::open(
                directory.path(),
                tenant(),
                &[Some(old.clone()), Some(pending.clone())],
                &CommitAuthority
            )
            .is_err()
        );
        child.0.kill()?;
        assert_eq!(child.0.wait()?.signal(), Some(9), "must be SIGKILL");
        let durable = matches!(boundary, "after_rename" | "after_directory_sync");
        if !durable {
            assert_eq!(fs::read(directory.path().join("journal.json"))?, original);
        }
        let scratch = fs::read(directory.path().join("journal.next")).ok();
        let reopened = Application::new(FileLedger::open(
            directory.path(),
            tenant(),
            &[Some(old), Some(pending)],
            &CommitAuthority,
        )?)?;
        if let Some(bytes) = scratch {
            assert_eq!(
                fs::read(directory.path().join("journal.interrupted"))?,
                bytes
            );
            assert!(!directory.path().join("journal.next").exists());
        }
        assert_eq!(
            reopened.ledger.records()?.len(),
            if durable { 2 } else { 1 }
        );
        let lookup = reopened
            .ledger
            .lookup_mutation(&tenant(), &request.intent.intent_id)
            .await?;
        if durable {
            assert!(matches!(lookup, LedgerLookup::Committed(_)));
            let RecoveryOutcome::Committed(receipt) = recover(
                &reopened.ledger,
                &reopened.policy,
                &CommitAuthority,
                &reopened.principal,
                &request.intent,
            )
            .await?
            else {
                return Err("durable write did not recover".into());
            };
            assert_eq!(receipt.commit(), plan.signed_commit());
            assert_eq!(receipt.record().events, plan.prepared().events());
            assert!(matches!(
                plan.commit(&reopened.ledger, &reopened.policy, &CommitAuthority)
                    .await?,
                DurableOutcome::Replay(_)
            ));
        } else {
            assert!(matches!(lookup, LedgerLookup::NotFound));
            assert!(matches!(
                plan.commit(&reopened.ledger, &reopened.policy, &CommitAuthority)
                    .await?,
                DurableOutcome::Committed(_)
            ));
        }
        let records = reopened.ledger.records()?;
        assert_eq!(records.len(), 2);
        for row in &records {
            assert_eq!(row.events.len(), 1);
            assert_eq!(row.projections.len(), 1);
            assert_eq!(row.outbox.len(), 1);
        }
        assert_eq!(records[0].commit, *initial.signed_commit());
        assert_eq!(records[1].commit, *plan.signed_commit());
        assert_eq!(records[1].events, plan.prepared().events());
        let resource = ResourceId(String::from("configuration:main"));
        let historical = reopened
            .ledger
            .state_proof(&resource, &initial.signed_commit().body.commit_id)?;
        let current = reopened
            .ledger
            .state_proof(&resource, &plan.signed_commit().body.commit_id)?;
        let verifier = ClaimVerifier::new(Box::new(reopened.ledger.clone()));
        let expectation = ExpectedClaim::state(tenant(), resource);
        let key = trusted_commit_key();
        assert!(
            verifier
                .verify_current(ClaimProof::State(&historical), &expectation, &key)
                .await
                .is_err()
        );
        verifier
            .verify_historical(
                ClaimProof::State(&historical),
                &expectation,
                &initial.signed_commit().body.commit_id,
                &key,
            )
            .await?;
        verifier
            .verify_current(ClaimProof::State(&current), &expectation, &key)
            .await?;
        let unchanged = fs::read(directory.path().join("journal.json"))?;
        assert!(matches!(
            plan.commit(&reopened.ledger, &reopened.policy, &CommitAuthority)
                .await?,
            DurableOutcome::Replay(_)
        ));
        let mut conflict = request.intent.clone();
        conflict.nonce = statechronicle::domain::intent::Nonce::from_bytes(vec![2])?;
        assert!(matches!(
            recover(
                &reopened.ledger,
                &reopened.policy,
                &CommitAuthority,
                &reopened.principal,
                &conflict
            )
            .await,
            Err(DurableError::PayloadConflict)
        ));
        assert_eq!(fs::read(directory.path().join("journal.json"))?, unchanged);
    }
    Ok(())
}

#[tokio::test]
async fn scratch_recovery_preserves_evidence_and_refuses_invalid_authority_or_occupied_slot()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (app, _, plan) = genesis(&directory).await?;
    plan.commit(&app.ledger, &app.policy, &CommitAuthority)
        .await?;
    let pin = tip(&plan)?;
    drop(app);
    let scratch = directory.path().join("journal.next");
    fs::write(&scratch, b"partial uncommitted bytes")?;
    // Wrong independent pins must not cause recovery to move any evidence.
    assert!(FileLedger::open(directory.path(), tenant(), &[None], &CommitAuthority).is_err());
    assert_eq!(fs::read(&scratch)?, b"partial uncommitted bytes");
    assert!(!directory.path().join("journal.interrupted").exists());
    fs::write(
        directory.path().join("journal.interrupted"),
        b"previous evidence",
    )?;
    assert!(
        FileLedger::open(
            directory.path(),
            tenant(),
            &[Some(pin.clone())],
            &CommitAuthority
        )
        .is_err()
    );
    assert_eq!(fs::read(&scratch)?, b"partial uncommitted bytes");
    assert_eq!(
        fs::read(directory.path().join("journal.interrupted"))?,
        b"previous evidence"
    );
    // An operator archives the occupied evidence slot before a new recovery.
    fs::rename(
        directory.path().join("journal.interrupted"),
        directory.path().join("archived"),
    )?;
    let reopened = FileLedger::open(directory.path(), tenant(), &[Some(pin)], &CommitAuthority)?;
    assert_eq!(reopened.records()?.len(), 1);
    assert_eq!(
        fs::read(directory.path().join("journal.interrupted"))?,
        b"partial uncommitted bytes"
    );
    Ok(())
}
