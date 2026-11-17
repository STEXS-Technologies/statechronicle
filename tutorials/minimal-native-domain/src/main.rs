//! Signed, authenticated preparation; deliberately no ledger or durable success.
mod domains;
mod host;

use domains::{CONFIGURATION, ConfigurationCommand, EQUIPMENT, EquipmentCommand};
use host::Application;
use statechronicle::executor::pipeline::PlayerBatchItem;
use std::error::Error;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let application = Application::development()?;
    let configuration = application.request(
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: true },
    )?;
    let equipment = application.request("equipment", &*EQUIPMENT, &EquipmentCommand::Install)?;
    let items = [
        PlayerBatchItem {
            validated: &configuration,
            principal: &application.principal,
        },
        PlayerBatchItem {
            validated: &equipment,
            principal: &application.principal,
        },
    ];
    let prepared = application
        .preparer
        .prepare_player_batch(&items, &application.policy, &application.keys)
        .await?;
    let mut events = prepared.events();
    let configuration = events.next().ok_or("missing configuration event")?;
    let equipment = events.next().ok_or("missing equipment event")?;
    println!(
        "Prepared configuration: {:?}",
        CONFIGURATION.decode_state(&configuration.after.state)?
    );
    println!(
        "Prepared equipment: {:?}",
        EQUIPMENT.decode_state(&equipment.after.state)?
    );
    println!("Uncommitted: no intent claim, ledger write, outbox dispatch or durable receipt.");
    Ok(())
}

#[cfg(test)]
mod tests;
