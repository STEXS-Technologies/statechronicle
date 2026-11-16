use super::*;
use domains::{ConfigurationState, EquipmentMode, EquipmentState};
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::ports::authorization::AuthenticatedPrincipal;
use statechronicle::profiles::registry::ProfileRules;
use statechronicle::{Operation, StateType};

#[tokio::test]
async fn signed_native_domains_prepare_and_recompute_their_own_transitions()
-> Result<(), Box<dyn Error>> {
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
    let batch = application
        .preparer
        .prepare_player_batch(&items, &application.policy, &application.keys)
        .await?;
    assert_eq!(batch.mutations().len(), 2);
    let mut events = batch.events();
    let config_event = events.next().ok_or("missing configuration")?;
    let equipment_event = events.next().ok_or("missing equipment")?;
    assert!(events.next().is_none());
    assert_eq!(
        CONFIGURATION.decode_state(&config_event.after.state)?,
        ConfigurationState {
            revision: 1,
            enabled: true
        }
    );
    assert_eq!(
        EQUIPMENT.decode_state(&equipment_event.after.state)?,
        EquipmentState {
            mode: EquipmentMode::Idle,
            cycles: 0
        }
    );
    CONFIGURATION.verify_transition(None, &configuration.intent, config_event)?;
    EQUIPMENT.verify_transition(None, &equipment.intent, equipment_event)?;
    // Domain transition checks are independent of host signer/history trust.
    let mut rewritten = config_event.clone();
    rewritten.after.state_hash = equipment_event.after.state_hash.clone();
    assert!(
        CONFIGURATION
            .verify_transition(None, &configuration.intent, &rewritten)
            .is_err()
    );
    assert!(EQUIPMENT.decode_state(&config_event.after.state).is_err());
    assert!(
        CONFIGURATION
            .decode_state(&equipment_event.after.state)
            .is_err()
    );
    assert_eq!(
        canonicalize(&configuration.intent)?,
        canonicalize(
            &batch
                .mutations()
                .first()
                .ok_or("missing leg")?
                .validated()
                .intent
        )?
    );
    Ok(())
}

#[tokio::test]
async fn host_policy_denies_unknown_credentials_resources_operations_and_actor()
-> Result<(), Box<dyn Error>> {
    let application = Application::development()?;
    let request = application.request(
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: true },
    )?;
    let mut unauthorized = application.principal.clone();
    unauthorized.credential_id = String::from("unknown-session");
    assert!(
        application
            .preparer
            .prepare_player(
                &request,
                &unauthorized,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    let other_actor = AuthenticatedPrincipal {
        subject: statechronicle::SubjectId(String::from("account:other")),
        ..application.principal.clone()
    };
    assert!(
        application
            .preparer
            .prepare_player(
                &request,
                &other_actor,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    let wrong_resource = application.request(
        "equipment",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: true },
    )?;
    assert!(
        application
            .preparer
            .prepare_player(
                &wrong_resource,
                &application.principal,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    let forbidden = application.request(
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::SetEnabled { enabled: false },
    )?;
    assert!(
        application
            .preparer
            .prepare_player(
                &forbidden,
                &application.principal,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    // Refusal of one batch leg returns no successful group or subset.
    let items = [
        PlayerBatchItem {
            validated: &request,
            principal: &application.principal,
        },
        PlayerBatchItem {
            validated: &wrong_resource,
            principal: &application.principal,
        },
    ];
    assert!(
        application
            .preparer
            .prepare_player_batch(&items, &application.policy, &application.keys)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn signature_presence_body_binding_and_key_revocation_fail_closed()
-> Result<(), Box<dyn Error>> {
    let application = Application::development()?;
    let request = application.request(
        "configuration",
        &*CONFIGURATION,
        &ConfigurationCommand::Create { enabled: true },
    )?;
    let mut unsigned = request.clone();
    unsigned.signature = None;
    assert!(
        application
            .preparer
            .prepare_player(
                &unsigned,
                &application.principal,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    let mut rewritten = request.clone();
    // This remains in the permitted resource and operation scope, so signature
    // verification must reject a changed native payload, rather than policy alone.
    rewritten.intent.inputs = CONFIGURATION
        .encode_command(&ConfigurationCommand::Create { enabled: false })?
        .1;
    assert!(
        application
            .preparer
            .prepare_player(
                &rewritten,
                &application.principal,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    application.keys.revoke(
        &request
            .signature
            .as_ref()
            .ok_or("missing signature")?
            .key_id,
    )?;
    assert!(
        application
            .preparer
            .prepare_player(
                &request,
                &application.principal,
                &application.policy,
                &application.keys
            )
            .await
            .is_err()
    );
    Ok(())
}

#[test]
fn schema_and_signed_operation_cannot_reinterpret_native_command() -> Result<(), Box<dyn Error>> {
    let (operation, mut inputs) =
        CONFIGURATION.encode_command(&ConfigurationCommand::Create { enabled: true })?;
    let registry = domains::registry()?;
    let rules = registry.resolve(StateType::Custom, &inputs)?;
    assert!(
        rules
            .evaluate(None, &Operation::from_static("equipment.install"), &inputs)
            .is_err()
    );
    assert!(
        rules
            .evaluate(
                None,
                &Operation::from_static("configuration.set_enabled"),
                &inputs
            )
            .is_err()
    );
    inputs.insert(String::from("schema_version"), 2.into());
    assert!(registry.resolve(StateType::Custom, &inputs).is_err());
    assert!(CONFIGURATION.evaluate(None, &operation, &inputs).is_err());
    Ok(())
}
