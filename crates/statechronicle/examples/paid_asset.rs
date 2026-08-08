//! Run: `cargo run -p statechronicle --example paid_asset`
//!
//! Durable paid ownership under the `paid_unique_asset` overlay (protocol §20.3),
//! injected into the registry the way a real deployment registers a custom
//! profile: `ProfileRegistry::with_unique_asset(&PaidUniqueAssetRules)`.
//!
//! Shows that a studio cannot transfer or burn a buyer-owned asset without the
//! buyer's consent, that restriction preserves the owner (legal_hold), and that
//! `asset.hard_delete` is forbidden without consent and tombstones with it.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use serde_json::json;

use statechronicle::domain::authority::AuthorityProof;
use statechronicle::domain::event::Event;
use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::profiles::error::ProfileError;
use statechronicle::profiles::paid_unique_asset::PaidUniqueAssetRules;
use statechronicle::profiles::registry::ProfileRegistry;

use common::Harness;

const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
const STUDIO: &str = "service:studio";
const RESOURCE: &str = "asset:paid_001";

/// Builds a signed `asset.*` intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    id: &str,
    op: &'static str,
    actor: &str,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
) -> ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(actor)))
        .resource(ResourceId(String::from(RESOURCE)))
        .state_type(StateType::UniqueAsset)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), authority)
}

async fn apply(harness: &Harness, events: &mut Vec<Event>, intent: &ValidatedIntent) {
    let event = harness.run(intent, StateType::UniqueAsset).await;
    events.push(event);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // A real deployment registers a custom profile the same way: inject the
    // rule set into a registry and hand that to the executor.
    let harness = Harness::with_registry(ProfileRegistry::with_unique_asset(&PaidUniqueAssetRules));
    let mut events: Vec<Event> = Vec::new();

    println!("== paid_asset: durable paid ownership (paid_unique_asset overlay) ==");

    // mint(ALICE): version 0 -> 1, owner ALICE.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "paid_mint",
            "asset.mint",
            ALICE,
            0,
            &[("to_owner", json!(ALICE))],
            None,
        ),
    )
    .await;
    println!("mint(ALICE)          -> {}", events[0].after.state);

    // Studio transfer without buyer consent fails (OwnershipMismatch).
    let studio_transfer = signed(
        &harness,
        "paid_studio_transfer",
        "asset.transfer",
        STUDIO,
        1,
        &[
            ("actor", json!(STUDIO)),
            ("from_owner", json!(ALICE)),
            ("to_owner", json!(BOB)),
        ],
        Some(harness.authority()),
    );
    let err = harness
        .executor
        .execute(&studio_transfer)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ExecutorError::Profile(ProfileError::OwnershipMismatch { .. })
    ));
    println!("studio transfer no consent -> rejected (OwnershipMismatch)");

    // Same transfer with the owner's explicit consent succeeds: owner -> BOB.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "paid_transfer",
            "asset.transfer",
            STUDIO,
            1,
            &[
                ("actor", json!(STUDIO)),
                ("from_owner", json!(ALICE)),
                ("to_owner", json!(BOB)),
                ("authorized_by_owner", json!(true)),
            ],
            Some(harness.authority()),
        ),
    )
    .await;
    assert_eq!(events[1].after.state["owner"], json!(BOB));
    println!("studio transfer with consent -> {}", events[1].after.state);

    // Studio burn of a buyer-owned asset fails closed (owner mismatch).
    let studio_burn = signed(
        &harness,
        "paid_studio_burn",
        "asset.burn",
        STUDIO,
        2,
        &[("from_owner", json!(STUDIO))],
        Some(harness.authority()),
    );
    let burn_err = harness.executor.execute(&studio_burn).await.unwrap_err();
    assert!(matches!(burn_err, ExecutorError::ActorMismatch { .. }));
    println!("studio burn of buyer-owned -> rejected (ownership mismatch)");

    // restrict(status=legal_hold) preserves the owner (append-only exceptional
    // states). restrict is authority-required in the paid overlay.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "paid_restrict",
            "asset.restrict",
            BOB,
            2,
            &[("status", json!("legal_hold"))],
            Some(harness.authority()),
        ),
    )
    .await;
    assert_eq!(events[2].after.state["owner"], json!(BOB));
    assert_eq!(events[2].after.state["status"], json!("legal_hold"));
    println!(
        "restrict(legal_hold)  -> {} (owner kept)",
        events[2].after.state
    );

    // restore returns the asset to active.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "paid_restore",
            "asset.restore",
            BOB,
            3,
            &[],
            Some(harness.authority()),
        ),
    )
    .await;
    assert_eq!(events[3].after.state["status"], json!("active"));
    println!("restore               -> {}", events[3].after.state);

    // hard_delete without consent is forbidden (HardDeleteForbidden).
    let hard_delete_no_consent = signed(
        &harness,
        "paid_hard_delete_no",
        "asset.hard_delete",
        BOB,
        4,
        &[("actor", json!(BOB))],
        Some(harness.authority()),
    );
    let hard_delete_err = harness
        .executor
        .execute(&hard_delete_no_consent)
        .await
        .unwrap_err();
    assert!(matches!(
        hard_delete_err,
        ExecutorError::Profile(ProfileError::HardDeleteForbidden)
    ));
    println!("hard_delete no consent -> rejected (HardDeleteForbidden)");

    // hard_delete with the owner's consent tombstones (terminal).
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "paid_hard_delete",
            "asset.hard_delete",
            BOB,
            4,
            &[("actor", json!(BOB)), ("authorized_by_owner", json!(true))],
            Some(harness.authority()),
        ),
    )
    .await;
    assert_eq!(events[4].after.state["status"], json!("tombstoned"));
    println!(
        "hard_delete with consent -> {} (terminal)",
        events[4].after.state["status"]
    );

    // Build the signed commit + accumulator over every emitted event.
    let (signed, accumulator) = harness.commit_events(&events);
    println!(
        "commit {} signs {} events; state root {}",
        signed.body.commit_id.as_str(),
        events.len(),
        signed.body.next_state_root.as_str()
    );
    assert_eq!(
        accumulator.root().as_bytes(),
        signed.body.next_state_root.as_bytes()
    );

    println!("paid_asset: OK");
}
