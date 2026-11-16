//! Consumer-controlled schema migration and recovery against the transaction fixture.
use super::*;
use serde::{Deserialize, Serialize};
use statechronicle::TypedProfile;
use std::sync::LazyLock;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct VersionedConfiguration {
    enabled: bool,
    source_revision: u32,
    updates: u32,
}
#[derive(Serialize, Deserialize)]
enum VersionedCommand {
    Migrate { enabled: bool, source_revision: u32 },
    SetEnabled { enabled: bool },
}
struct ConfigurationV2;
impl StateProfile for ConfigurationV2 {
    type State = VersionedConfiguration;
    type Command = VersionedCommand;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = Configuration::DOMAIN_ID;
    const SCHEMA_VERSION: u32 = 2;
    const MAX_PAYLOAD_BYTES: usize = 64;
    fn operation(&self, command: &Self::Command) -> &'static str {
        match command {
            VersionedCommand::Migrate { .. } => "configuration.migrate",
            VersionedCommand::SetEnabled { .. } => "configuration.set_enabled",
        }
    }
    fn validate(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<(), Self::Error> {
        match (current, command) {
            (
                None,
                VersionedCommand::Migrate {
                    source_revision, ..
                },
            ) if *source_revision > 0 => Ok(()),
            (Some(_), VersionedCommand::SetEnabled { .. }) => Ok(()),
            _ => Err("invalid migration lifecycle"),
        }
    }
    fn apply(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<Self::State, Self::Error> {
        self.validate(current, command)?;
        match (current, command) {
            (
                None,
                VersionedCommand::Migrate {
                    enabled,
                    source_revision,
                },
            ) => Ok(VersionedConfiguration {
                enabled: *enabled,
                source_revision: *source_revision,
                updates: 0,
            }),
            (Some(state), VersionedCommand::SetEnabled { enabled }) => Ok(VersionedConfiguration {
                enabled: *enabled,
                source_revision: state.source_revision,
                updates: state.updates.checked_add(1).ok_or("update overflow")?,
            }),
            _ => Err("invalid migration lifecycle"),
        }
    }
    fn validate_state(&self, state: &Self::State) -> Result<(), Self::Error> {
        if state.source_revision == 0 {
            Err("missing migration revision")
        } else {
            Ok(())
        }
    }
}
static OPERATIONS: LazyLock<[Operation; 2]> = LazyLock::new(|| {
    [
        Operation::new(String::from("configuration.migrate")).unwrap(),
        Operation::new(String::from("configuration.set_enabled")).unwrap(),
    ]
});
static V2: LazyLock<TypedProfile<ConfigurationV2>> =
    LazyLock::new(|| TypedProfile::new(ConfigurationV2, &*OPERATIONS));
static PROFILES: LazyLock<[&'static dyn ProfileRules; 2]> =
    LazyLock::new(|| [&*CONFIGURATION, &*V2]);

#[tokio::test]
async fn schema_migration_recovers_versions_after_lost_commit_acknowledgement() {
    let mut harness = Harness::with_registry(ProfileRegistry::with_custom(&*PROFILES).unwrap());
    let mut ledger = MemoryLedger::default();
    let mut history: Vec<PreparedMutation> = Vec::new();
    let mut tree = StateAccumulator::empty();
    let mut parent = None;
    let mut previous_root = ContentDigest::new(*tree.root().as_bytes());
    for step in 0_u64..4 {
        let raw = match step {
            0 => payload(
                "int_v1_create",
                "config:version1",
                0,
                &Configuration,
                &ConfigurationCommand::Create { enabled: true },
            ),
            1 => payload(
                "int_v1_update",
                "config:version1",
                1,
                &Configuration,
                &ConfigurationCommand::SetEnabled { enabled: false },
            ),
            2 => {
                let old = CONFIGURATION
                    .decode_state(&history[1_usize].event.after.state)
                    .unwrap();
                let command = VersionedCommand::Migrate {
                    enabled: old.enabled,
                    source_revision: old.revision,
                };
                let forbidden = harness.accept(
                    &payload(
                        "int_in_place",
                        "config:version1",
                        2,
                        &ConfigurationV2,
                        &command,
                    ),
                    Some(harness.authority()),
                );
                assert!(harness.executor.execute(&forbidden).await.is_err());
                payload(
                    "int_v2_migrate",
                    "config:version2",
                    0,
                    &ConfigurationV2,
                    &command,
                )
            }
            _ => payload(
                "int_v2_update",
                "config:version2",
                1,
                &ConfigurationV2,
                &VersionedCommand::SetEnabled { enabled: true },
            ),
        };
        let accepted = harness.accept(&raw, Some(harness.authority()));
        let event = harness.run(&accepted, StateType::Custom).await;
        let (mut commit, _) = harness.commit_events(from_ref(&event));
        tree.insert_batch(&state_root_updates(from_ref(&event)).unwrap())
            .unwrap();
        commit.body.commit_id = CommitId::new(format!("cmt_versioned_{step}")).unwrap();
        commit.body.sequence = step + 1;
        commit.body.parent_commit_id = parent;
        commit.body.previous_state_root = previous_root;
        commit.body.next_state_root = ContentDigest::new(*tree.root().as_bytes());
        parent = Some(commit.body.commit_id.clone());
        previous_root = commit.body.next_state_root.clone();
        commit = sign_commit(&commit.body, &fixed_key(), harness.key_id()).unwrap();
        let mutation = PreparedMutation {
            accepted,
            event,
            commit,
        };
        let principal = AuthenticatedPrincipal {
            tenant: harness.tenant(),
            subject: mutation.accepted.intent.actor.clone(),
            credential_id: String::from("versioned-recovery"),
        };
        ledger.lose_commit_response = step == 2;
        let result = deliver_prepared(&ledger, &principal, &mutation).await;
        if step == 2 {
            assert!(result.is_err());
            ledger = MemoryLedger {
                data: ledger.data.clone(),
                ..MemoryLedger::default()
            };
            assert!(
                deliver_prepared(&ledger, &principal, &mutation)
                    .await
                    .is_ok()
            );
            harness = Harness::with_registry(ProfileRegistry::with_custom(&*PROFILES).unwrap());
            let stored = ledger.data.lock().unwrap().clone();
            assert_eq!(stored.events.len(), 3);
            for persisted_event in &stored.events {
                harness.index.apply(persisted_event, StateType::Custom);
            }
            let stale = harness.accept(
                &payload(
                    "int_v2_stale",
                    "config:version2",
                    0,
                    &ConfigurationV2,
                    &VersionedCommand::SetEnabled { enabled: true },
                ),
                Some(harness.authority()),
            );
            assert!(harness.executor.execute(&stale).await.is_err());
        } else {
            assert!(result.is_ok());
        }
        history.push(mutation);
    }
    let stored = ledger.data.lock().unwrap().clone();
    assert_eq!(stored.commits.len(), 4);
    assert_eq!(stored.events.len(), 4);
    assert_eq!(stored.identities.len(), 4);
    for (persisted, mutation) in stored.events.iter().zip(&history) {
        assert_eq!(persisted, &mutation.event);
    }
    for (persisted, mutation) in stored.commits.iter().zip(&history) {
        assert_eq!(persisted, &mutation.commit);
    }
    let mut projections = BTreeMap::new();
    let mut replayed = StateAccumulator::empty();
    for mutation in &history {
        let current = projections.get(&mutation.event.resource_id);
        let ResourceState::Custom(ref state) = mutation.event.after.state else {
            panic!("expected custom domain state");
        };
        match state.schema_version {
            1 => CONFIGURATION
                .verify_transition(current, &mutation.accepted.intent, &mutation.event)
                .unwrap(),
            2 => V2
                .verify_transition(current, &mutation.accepted.intent, &mutation.event)
                .unwrap(),
            _ => panic!("unexpected schema version"),
        }
        let entries = [CommittedEvent {
            event: &mutation.event,
            state_type: StateType::Custom,
        }];
        let projection = projections_for(&mutation.commit.body.commit_id, &entries).remove(0);
        projections.insert(mutation.event.resource_id.clone(), projection);
        replayed
            .insert_batch(&state_root_updates(from_ref(&mutation.event)).unwrap())
            .unwrap();
    }
    assert_eq!(replayed.root(), tree.root());
    let old = CONFIGURATION
        .decode_state(&stored.events[1].after.state)
        .unwrap();
    assert_eq!(old.revision, 2);
    let migrated = V2.decode_state(&stored.events[3].after.state).unwrap();
    assert_eq!(
        migrated,
        VersionedConfiguration {
            enabled: true,
            source_revision: 2,
            updates: 1
        }
    );
    assert!(
        CONFIGURATION
            .decode_state(&stored.events[3].after.state)
            .is_err()
    );
    assert!(V2.decode_state(&stored.events[1].after.state).is_err());
}
