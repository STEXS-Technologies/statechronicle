//! File-backed custom ledger, unknown-ACK recovery, native replay and proof expectations.
mod domains;
mod host;
mod ledger;

use domains::{CONFIGURATION, ConfigurationCommand, EQUIPMENT, EquipmentCommand};
use host::{Application, CommitAuthority, tenant, trusted_commit_key};
use ledger::FileLedger;
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::mutation::{DurableError, DurableOutcome, RecoveryOutcome, recover};
use statechronicle::proof::claim::{ClaimProof, ClaimVerifier, ExpectedClaim};
use statechronicle::{ContentDigest, ResourceId};
use std::error::Error;

fn tip(plan: &statechronicle::mutation::SignedMutation) -> Result<ContentDigest, Box<dyn Error>> {
    Ok(canonicalize_and_digest(&plan.signed_commit().body)?)
}

async fn demonstrate() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let ledger = FileLedger::open(directory.path(), tenant(), &[None], &CommitAuthority)?;
    let application = Application::new(ledger)?;
    let request = application.request(
        "int_configuration_create",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: true },
    )?;
    let plan = application.plan(&request).await?;
    let independently_retained_tip = tip(&plan)?;
    let unknown = plan
        .commit(
            &application.ledger.lose_next_ack(),
            &application.policy,
            &CommitAuthority,
        )
        .await;
    if !matches!(unknown, Err(DurableError::Unknown { .. })) {
        return Err("expected unknown acknowledgement".into());
    }
    println!("Commit acknowledgement: unknown; retain original identity and signed plan.");
    // Release every adapter handle and writer lock before reopening from disk.
    drop(application);
    let ledger = FileLedger::open(
        directory.path(),
        tenant(),
        &[None, Some(independently_retained_tip)],
        &CommitAuthority,
    )?;
    let application = Application::new(ledger)?;
    let recovered = recover(
        &application.ledger,
        &application.policy,
        &CommitAuthority,
        &application.principal,
        &request.intent,
    )
    .await?;
    let original = match recovered {
        RecoveryOutcome::Committed(receipt) => receipt,
        _ => return Err("unresolved original identity; do not blindly retry".into()),
    };
    println!("Recovery after reopen: original committed mutation verified.");
    if !matches!(
        plan.commit(&application.ledger, &application.policy, &CommitAuthority)
            .await?,
        DurableOutcome::Replay(_)
    ) {
        return Err("expected exact replay".into());
    }
    println!("Exact retry: replay; one original event, commit and outbox row.");
    let resource = ResourceId(String::from("configuration:main"));
    let historical_pin = original.commit().body.commit_id.clone();
    let old_proof = application.ledger.state_proof(&resource, &historical_pin)?;
    let update = application.request(
        "int_configuration_update",
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::SetEnabled { enabled: false },
    )?;
    let update_plan = application.plan(&update).await?;
    update_plan
        .commit(&application.ledger, &application.policy, &CommitAuthority)
        .await?;
    let equipment = application.request(
        "int_equipment_install",
        "equipment",
        &*EQUIPMENT,
        &EquipmentCommand::Install,
    )?;
    application
        .plan(&equipment)
        .await?
        .commit(&application.ledger, &application.policy, &CommitAuthority)
        .await?;
    let start = application.request(
        "int_equipment_start",
        "equipment",
        &*EQUIPMENT,
        &EquipmentCommand::Start,
    )?;
    let latest = application.plan(&start).await?;
    latest
        .commit(&application.ledger, &application.policy, &CommitAuthority)
        .await?;
    let verifier = ClaimVerifier::new(Box::new(application.ledger.clone()));
    let expectation = ExpectedClaim::state(tenant(), resource.clone());
    let trusted = trusted_commit_key();
    if verifier
        .verify_current(ClaimProof::State(&old_proof), &expectation, &trusted)
        .await
        .is_ok()
    {
        return Err("stale proof accepted as current".into());
    }
    let historical = verifier
        .verify_historical(
            ClaimProof::State(&old_proof),
            &expectation,
            &historical_pin,
            &trusted,
        )
        .await?;
    let current_proof = application
        .ledger
        .state_proof(&resource, &latest.signed_commit().body.commit_id)?;
    let current = verifier
        .verify_current(ClaimProof::State(&current_proof), &expectation, &trusted)
        .await?;
    println!(
        "Historical configuration: {:?}",
        CONFIGURATION.decode_state(historical.state().ok_or("missing historical state")?)?
    );
    println!(
        "Current configuration: {:?}",
        CONFIGURATION.decode_state(current.state().ok_or("missing current state")?)?
    );
    println!("Stale current proof: refused; independently pinned historical proof: verified.");
    let retained = application.ledger.records()?;
    if retained.len() != 4
        || retained.iter().map(|row| row.events.len()).sum::<usize>() != 4
        || retained.iter().map(|row| row.outbox.len()).sum::<usize>() != 4
    {
        return Err("unexpected retained effect counts".into());
    }
    println!("File journal: 4 commits; no notification dispatched.");
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    demonstrate().await
}

#[cfg(test)]
mod tests;
