//! Public built-in check/reduction trace; compiles against the original release.
#![allow(clippy::unwrap_used, clippy::too_many_lines)]
mod profile_guards;
use profile_guards::Case;
use serde_json::{Value, json};
use statechronicle::core::{amount::Amount, canonicalize::canonicalize_and_digest};
use statechronicle::domain::{
    commit::ProfileId,
    custom::CustomState,
    resource_state::{
        ConsumableStackState, EntitlementState, EscrowState, FungibleBalanceState, ListingState,
        MeterState, ResourceState, UniqueAssetState,
    },
    status::Status,
};
use statechronicle::profiles::{error::ProfileError, registry::ProfileRules, transition};
use statechronicle::{
    CommitId, EventId, Operation, ProfileRegistry, ResourceId, StateProjection, StateType,
    SubjectId, TenantId,
};
use std::collections::BTreeMap;

fn projection(kind: StateType, state: ResourceState) -> StateProjection {
    StateProjection {
        tenant_id: TenantId("tenant:characterization".into()),
        resource_id: ResourceId("resource:characterization".into()),
        state_type: kind,
        version: 1,
        last_event_id: EventId("evt_original".into()),
        last_commit_id: CommitId("cmt_original".into()),
        state_hash: canonicalize_and_digest(&state).unwrap(),
        state,
    }
}

fn payloads() -> Vec<ResourceState> {
    let mut states = Vec::new();
    for text in ["account:alice", ""] {
        let subject = SubjectId(text.into());
        for amount in [
            Amount::ZERO,
            Amount::from_u64(5),
            Amount::from_u128(u128::MAX),
        ] {
            states.extend([
                ResourceState::ConsumableStack(ConsumableStackState {
                    subject: subject.clone(),
                    quantity: amount,
                    unit: text.into(),
                }),
                ResourceState::FungibleBalance(FungibleBalanceState {
                    subject: subject.clone(),
                    balance: amount,
                    unit: text.into(),
                }),
                ResourceState::MeteredResource(MeterState {
                    subject: subject.clone(),
                    remaining: amount,
                    maximum: Amount::from_u64(5),
                }),
            ]);
        }
        for status in ["active", "granted", "trade_held", "listed", "held", ""] {
            states.extend([
                ResourceState::UniqueAsset(UniqueAssetState {
                    owner: subject.clone(),
                    status: Status::from_static(status),
                    trade_id: Some("trade_original".into()),
                }),
                ResourceState::Listing(ListingState {
                    seller: subject.clone(),
                    status: Status::from_static(status),
                }),
                ResourceState::Escrow(EscrowState {
                    buyer: subject.clone(),
                    seller: subject.clone(),
                    status: Status::from_static(status),
                }),
            ]);
            for transferable in [false, true] {
                states.push(ResourceState::Entitlement(EntitlementState {
                    subject: subject.clone(),
                    status: Status::from_static(status),
                    transferable,
                }));
            }
        }
    }
    states.push(ResourceState::Custom(CustomState {
        domain: ProfileId("test.custom".into()),
        schema_version: 1,
        payload: vec![1],
    }));
    states
}

fn input_cases() -> Vec<BTreeMap<String, Value>> {
    let mut base = BTreeMap::new();
    for key in ["subject", "from_owner", "authorized_by", "seller", "actor"] {
        base.insert(key.into(), json!("account:alice"));
    }
    for key in ["to_subject", "to_owner", "buyer"] {
        base.insert(key.into(), json!("account:bob"));
    }
    for key in ["quantity", "amount", "balance", "remaining", "maximum"] {
        base.insert(key.into(), json!("1"));
    }
    base.insert("unit".into(), json!("units"));
    base.insert("to_unit".into(), json!("other_units"));
    base.insert("trade_id".into(), json!("trade_original"));
    base.insert("transferable".into(), json!(true));
    let mut cases = vec![base.clone(), BTreeMap::new()];
    for key in [
        "subject",
        "to_subject",
        "to_owner",
        "buyer",
        "seller",
        "unit",
        "to_unit",
    ] {
        for replacement in [None, Some(json!("")), Some(json!(false)), Some(Value::Null)] {
            let mut inputs = base.clone();
            if let Some(value) = replacement {
                inputs.insert(key.into(), value);
            } else {
                inputs.remove(key);
            }
            cases.push(inputs);
        }
    }
    for amount in [
        json!("0"),
        json!("5"),
        json!("6"),
        json!("0001"),
        json!("-1"),
        json!("1.0"),
        json!("1e0"),
        json!(1),
        json!(false),
        json!(""),
        json!(u128::MAX.to_string()),
        json!("340282366920938463463374607431768211456"),
    ] {
        let mut inputs = base.clone();
        for key in ["quantity", "amount", "balance", "remaining", "maximum"] {
            inputs.insert(key.into(), amount.clone());
        }
        cases.push(inputs);
    }
    cases
}

fn error(error: ProfileError) -> Value {
    json!({"debug": format!("{error:?}"), "display": error.to_string()})
}
fn state(result: Result<ResourceState, ProfileError>) -> Value {
    match result {
        Ok(state) => json!({"ok": state, "digest": canonicalize_and_digest(&state).unwrap()}),
        Err(e) => json!({"error": error(e)}),
    }
}
fn trace(case: &Case, rules: &dyn ProfileRules) -> Value {
    let check = rules.check(&case.operation, case.current.as_ref(), &case.inputs);
    let checked = match check {
        Ok(()) => {
            json!({"apply": state(rules.apply(case.current.as_ref(), &case.operation, &case.inputs))})
        }
        Err(e) => json!({"check_error": error(e)}),
    };
    let mut row = json!({
        "case": case.label,
        "checked": checked,
        "apply": state(rules.apply(case.current.as_ref(), &case.operation, &case.inputs)),
        "direct": state(transition::apply(case.current.as_ref(), &case.operation, &case.inputs)),
    });
    if let Some(source) = &case.current {
        if matches!(
            case.operation.as_str(),
            "stack.transfer" | "balance.transfer"
        ) {
            row["transfer_unborn"] = state(transition::transfer_after_state(
                source,
                None,
                &case.operation,
                &case.inputs,
            ));
            row["transfer_existing"] = state(transition::transfer_after_state(
                source,
                Some(source),
                &case.operation,
                &case.inputs,
            ));
        }
    }
    row
}

fn main() {
    // Keep the established 14,364-case validation corpus unchanged and observable.
    let _ = profile_guards::transcript;
    let registry = ProfileRegistry::baseline();
    let mut result = Vec::new();
    for case in profile_guards::cases() {
        result.push(trace(&case, registry.get(case.kind).unwrap()));
    }
    let payloads = payloads();
    let inputs = input_cases();
    for (profile, kind) in [
        ("asset", StateType::UniqueAsset),
        ("stack", StateType::ConsumableStack),
        ("balance", StateType::FungibleBalance),
        ("entitlement", StateType::Entitlement),
        ("meter", StateType::MeteredResource),
        ("listing", StateType::Listing),
        ("escrow", StateType::Escrow),
        ("paid", StateType::UniqueAsset),
    ] {
        let rules = if profile == "paid" {
            registry.paid_unique_asset()
        } else {
            registry.get(kind).unwrap()
        };
        let mut operations = rules.allowed_operations().to_vec();
        operations.push(Operation("unknown.operation".into()));
        for operation in operations {
            for (index, payload) in std::iter::once(None)
                .chain(payloads.iter().map(Some))
                .enumerate()
            {
                let current = payload.map(|state| projection(kind, state.clone()));
                for (input, inputs) in inputs.iter().enumerate() {
                    result.push(trace(
                        &Case {
                            label: format!(
                                "expanded/{profile}/{}/{index}/{input}",
                                operation.as_str()
                            ),
                            kind,
                            operation: operation.clone(),
                            current: current.clone(),
                            inputs: inputs.clone(),
                        },
                        rules,
                    ));
                }
            }
        }
    }
    println!("{}", Value::Array(result));
}
