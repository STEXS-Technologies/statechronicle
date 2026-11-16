//! Public validation corpus shared with an independently archived v0.2 producer.
use serde_json::{Value, json};
use statechronicle::core::{amount::Amount, canonicalize::canonicalize_and_digest};
use statechronicle::domain::{
    resource_state::{
        ConsumableStackState, EntitlementState, EscrowState, FungibleBalanceState, ListingState,
        ResourceState, UniqueAssetState,
    },
    status::Status,
};
use statechronicle::{
    CommitId, EventId, Operation, ProfileRegistry, ResourceId, StateProjection, StateType,
    SubjectId, TenantId,
};
use std::collections::BTreeMap;

pub struct Case {
    pub label: String,
    pub kind: StateType,
    pub operation: Operation,
    pub current: Option<StateProjection>,
    pub inputs: BTreeMap<String, Value>,
}
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
fn subject() -> SubjectId {
    SubjectId("account:alice".into())
}
fn inputs() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("subject".into(), json!("account:alice")),
        ("to_subject".into(), json!("account:bob")),
        ("from_owner".into(), json!("account:alice")),
        ("to_owner".into(), json!("account:bob")),
        ("authorized_by".into(), json!("account:alice")),
        ("unit".into(), json!("units")),
        ("to_unit".into(), json!("other_units")),
        ("buyer".into(), json!("account:bob")),
        ("seller".into(), json!("account:alice")),
        ("trade_id".into(), json!("trade_original")),
    ])
}
pub fn cases() -> Vec<Case> {
    let registry = ProfileRegistry::baseline();
    let amounts = [
        None,
        Some(json!("")),
        Some(json!("0")),
        Some(json!("1")),
        Some(json!("5")),
        Some(json!("6")),
        Some(json!("0001")),
        Some(json!("-1")),
        Some(json!("1.0")),
        Some(json!("1e0")),
        Some(json!(1.0)),
        Some(json!(1)),
        Some(Value::Null),
        Some(json!(true)),
        Some(json!(u128::MAX.to_string())),
        Some(json!("340282366920938463463374607431768211456")),
    ];
    let mut result = Vec::new();
    for kind in [StateType::ConsumableStack, StateType::FungibleBalance] {
        let rules = registry.get(kind).unwrap();
        let mut operations = rules.allowed_operations().to_vec();
        operations.push(Operation("unknown.operation".into()));
        let mut states = vec![None];
        for amount in [
            Amount::ZERO,
            Amount::from_u64(5),
            Amount::from_u128(u128::MAX),
            Amount::from_mantissa(5, 1).unwrap(),
        ] {
            let state = match kind {
                StateType::ConsumableStack => {
                    ResourceState::ConsumableStack(ConsumableStackState {
                        subject: subject(),
                        quantity: amount,
                        unit: "units".into(),
                    })
                }
                StateType::FungibleBalance => {
                    ResourceState::FungibleBalance(FungibleBalanceState {
                        subject: subject(),
                        balance: amount,
                        unit: "units".into(),
                    })
                }
                StateType::UniqueAsset
                | StateType::Entitlement
                | StateType::MeteredResource
                | StateType::Listing
                | StateType::Escrow
                | StateType::Custom => panic!("unsupported fixture kind {kind:?}"),
            };
            states.push(Some(projection(kind, state)));
        }
        states.push(Some(projection(
            kind,
            ResourceState::UniqueAsset(UniqueAssetState {
                owner: subject(),
                status: Status::from_static("active"),
                trade_id: None,
            }),
        )));
        for operation in operations {
            for (state_index, current) in states.iter().enumerate() {
                for (amount_index, amount) in amounts.iter().enumerate() {
                    for metadata in 0..6 {
                        let mut data = inputs();
                        if let Some(value) = amount {
                            for key in ["amount", "quantity", "balance"] {
                                data.insert(key.into(), value.clone());
                            }
                        }
                        match metadata {
                            1 => {
                                data.remove("to_subject");
                            }
                            2 => {
                                data.insert("to_subject".into(), json!(""));
                            }
                            3 => {
                                data.remove("authorized_by");
                            }
                            4 => {
                                data.insert("to_unit".into(), json!(false));
                            }
                            5 => {
                                data.retain(|key, _| key == "quantity" || key == "amount");
                            }
                            _ => {}
                        }
                        result.push(Case {
                            label: format!(
                                "{kind:?}/{}/{state_index}/{amount_index}/{metadata}",
                                operation.as_str()
                            ),
                            kind,
                            operation: operation.clone(),
                            current: current.clone(),
                            inputs: data,
                        });
                    }
                }
            }
        }
    }
    for kind in [
        StateType::UniqueAsset,
        StateType::Entitlement,
        StateType::Listing,
        StateType::Escrow,
    ] {
        let mut operations = registry.get(kind).unwrap().allowed_operations().to_vec();
        operations.push(Operation("unknown.operation".into()));
        let mut states = vec![None];
        for name in [
            "active",
            "locked",
            "listed",
            "escrowed",
            "trade_held",
            "redeemed",
            "burned",
            "tombstoned",
            "restricted",
            "quarantined",
            "unsupported",
            "granted",
            "suspended",
            "expired",
            "revoked",
            "released",
            "refunded",
            "",
            "other",
        ] {
            let status = Status::from_static(name);
            let state = match kind {
                StateType::UniqueAsset => ResourceState::UniqueAsset(UniqueAssetState {
                    owner: subject(),
                    status,
                    trade_id: Some("trade_original".into()),
                }),
                StateType::Entitlement => ResourceState::Entitlement(EntitlementState {
                    subject: subject(),
                    status,
                    transferable: true,
                }),
                StateType::Listing => ResourceState::Listing(ListingState {
                    seller: subject(),
                    status,
                }),
                StateType::Escrow => ResourceState::Escrow(EscrowState {
                    buyer: SubjectId("account:bob".into()),
                    seller: subject(),
                    status,
                }),
                StateType::ConsumableStack
                | StateType::FungibleBalance
                | StateType::MeteredResource
                | StateType::Custom => panic!("unsupported fixture kind {kind:?}"),
            };
            states.push(Some(projection(kind, state)));
        }
        // Existing projection with a payload that has no status field.
        states.push(Some(projection(
            kind,
            ResourceState::FungibleBalance(FungibleBalanceState {
                subject: subject(),
                balance: Amount::ZERO,
                unit: "units".into(),
            }),
        )));
        for operation in operations {
            for (state_index, current) in states.iter().enumerate() {
                for metadata in 0..3 {
                    let data = match metadata {
                        0 => inputs(),
                        1 => BTreeMap::new(),
                        _ => BTreeMap::from([("to_owner".into(), json!(false))]),
                    };
                    result.push(Case {
                        label: format!("{kind:?}/{}/{state_index}/{metadata}", operation.as_str()),
                        kind,
                        operation: operation.clone(),
                        current: current.clone(),
                        inputs: data,
                    });
                }
            }
        }
    }
    result
}
pub fn transcript() -> Value {
    let registry = ProfileRegistry::baseline();
    Value::Array(cases().into_iter().map(|case| {
        let checked = registry.get(case.kind).unwrap().check(&case.operation, case.current.as_ref(), &case.inputs);
        match checked {
            Ok(()) => json!({"case": case.label, "result": "Ok"}),
            Err(error) => json!({"case": case.label, "result": format!("{error:?}"), "display": error.to_string()}),
        }
    }).collect())
}
