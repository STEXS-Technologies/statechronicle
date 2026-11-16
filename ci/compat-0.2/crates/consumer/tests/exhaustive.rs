//! Frozen exhaustive matches against original public enums. Do not regenerate from candidate.
#![allow(dead_code)]

fn match_domain_state_type(value: statechronicle::domain::state_type::StateType) -> usize {
    use statechronicle::domain::state_type::StateType;
    match value {
        StateType::UniqueAsset => 0,
        StateType::ConsumableStack => 1,
        StateType::FungibleBalance => 2,
        StateType::Entitlement => 3,
        StateType::MeteredResource => 4,
        StateType::Listing => 5,
        StateType::Escrow => 6,
        StateType::Custom => 7,
    }
}

fn match_domain_proof(value: statechronicle::domain::proof::ProofType) -> usize {
    use statechronicle::domain::proof::ProofType;
    match value {
        ProofType::Inclusion => 0,
        ProofType::State => 1,
        ProofType::Transition => 2,
        ProofType::Ownership => 3,
        ProofType::History => 4,
        ProofType::Snapshot => 5,
        ProofType::Authority => 6,
        ProofType::Commit => 7,
        ProofType::NonMembership => 8,
    }
}

fn match_domain_error(value: statechronicle::domain::error::DomainError) -> usize {
    use statechronicle::domain::error::DomainError;
    match value {
        DomainError::InvalidId { .. } => 0,
        DomainError::InvalidOperation(..) => 1,
        DomainError::InvalidStatus(..) => 2,
        DomainError::InvalidKeyId(..) => 3,
        DomainError::InvalidProfile(..) => 4,
        DomainError::InvalidNonce(..) => 5,
        DomainError::BuilderFieldMissing(..) => 6,
        DomainError::Core(..) => 7,
    }
}

fn match_intent_error(value: statechronicle::intent::error::IntentError) -> usize {
    use statechronicle::intent::error::IntentError;
    match value {
        IntentError::SizeLimitExceeded { .. } => 0,
        IntentError::InvalidJson { .. } => 1,
        IntentError::InvalidSchema { .. } => 2,
        IntentError::InvalidField(..) => 3,
        IntentError::InvalidExpiry(..) => 4,
        IntentError::Domain(..) => 5,
    }
}

fn match_executor_error(value: statechronicle::executor::error::ExecutorError) -> usize {
    use statechronicle::executor::error::ExecutorError;
    match value {
        ExecutorError::Intent(..) => 0,
        ExecutorError::Domain(..) => 1,
        ExecutorError::Profile(..) => 2,
        ExecutorError::SizeLimitExceeded { .. } => 3,
        ExecutorError::Expired { .. } => 4,
        ExecutorError::DuplicateIntent { .. } => 5,
        ExecutorError::ExpectedVersionMismatch { .. } => 6,
        ExecutorError::TenantScopeMissing => 7,
        ExecutorError::TenantNotFound { .. } => 8,
        ExecutorError::ResourceNotFound { .. } => 9,
        ExecutorError::AuthorityDenied => 10,
        ExecutorError::AuthorityStale => 11,
        ExecutorError::AuthorityUnavailable(..) => 12,
        ExecutorError::AuthorityMissing { .. } => 13,
        ExecutorError::ActorMismatch { .. } => 14,
        ExecutorError::ResourceLocked { .. } => 15,
        ExecutorError::TransitionInvalid(..) => 16,
        ExecutorError::ActorAuthenticationFailed(..) => 17,
        ExecutorError::TransferMismatch(..) => 18,
        ExecutorError::AtomicityViolation(..) => 19,
        ExecutorError::ValueLegSettleRouting { .. } => 20,
        ExecutorError::Store(..) => 21,
        ExecutorError::StateTypeRequired => 22,
    }
}

fn match_commit_error(value: statechronicle::commit::error::CommitError) -> usize {
    use statechronicle::commit::error::CommitError;
    match value {
        CommitError::EmptyBatch => 0,
        CommitError::Domain(..) => 1,
        CommitError::Core(..) => 2,
        CommitError::Accumulator(..) => 3,
        CommitError::InvalidEvent(..) => 4,
        CommitError::MixedTenant => 5,
        CommitError::DuplicateEventId { .. } => 6,
        CommitError::DuplicateCanonicalKey { .. } => 7,
        CommitError::OutOfOrder { .. } => 8,
        CommitError::StateRootMismatch { .. } => 9,
        CommitError::EventRootMismatch => 10,
        CommitError::ForkDetected { .. } => 11,
        CommitError::ChainGap { .. } => 12,
        CommitError::SequenceMismatch { .. } => 13,
        CommitError::EventRewrite { .. } => 14,
        CommitError::SizeLimitExceeded { .. } => 15,
        CommitError::BuilderFieldMissing(..) => 16,
        CommitError::Signing(..) => 17,
        CommitError::PayloadDigestMismatch => 18,
        CommitError::Store(..) => 19,
    }
}

fn match_proof_error(value: statechronicle::proof::error::ProofError) -> usize {
    use statechronicle::proof::error::ProofError;
    match value {
        ProofError::Domain(..) => 0,
        ProofError::Core(..) => 1,
        ProofError::SizeLimitExceeded { .. } => 2,
        ProofError::NotFound => 3,
        ProofError::UnsupportedSchema(..) => 4,
        ProofError::UnsupportedKind(..) => 5,
        ProofError::InvalidState(..) => 6,
        ProofError::CommitSignature(..) => 7,
        ProofError::CommitRefMismatch { .. } => 8,
        ProofError::CommitScope(..) => 9,
        ProofError::NonCanonicalCommit { .. } => 10,
        ProofError::CanonicalHeadUnavailable => 11,
        ProofError::KeyNotFound(..) => 12,
        ProofError::TenantMismatch { .. } => 13,
        ProofError::ResourceMismatch { .. } => 14,
        ProofError::SubjectMismatch { .. } => 15,
        ProofError::MissingOwner => 16,
        ProofError::InclusionMismatch => 17,
        ProofError::ClaimedStateMismatch => 18,
        ProofError::LeafMismatch => 19,
        ProofError::InvalidPathLength { .. } => 20,
        ProofError::KeyMismatch { .. } => 21,
        ProofError::NonMembershipLeafMismatch => 22,
        ProofError::Store(..) => 23,
        ProofError::ProofIndex(..) => 24,
        ProofError::TradeIdMismatch { .. } => 25,
        ProofError::TradeMissingSide(..) => 26,
        ProofError::TradeOperation(..) => 27,
        ProofError::TradeProofTruncated { .. } => 28,
    }
}

fn match_profiles_error(value: statechronicle::profiles::error::ProfileError) -> usize {
    use statechronicle::profiles::error::ProfileError;
    match value {
        ProfileError::ActorMismatch { .. } => 0,
        ProfileError::ResourceLocked { .. } => 1,
        ProfileError::AtomicityViolation(..) => 2,
        ProfileError::TransferMismatch(..) => 3,
        ProfileError::ValueLegSettleRouting { .. } => 4,
        ProfileError::UnknownOperation(..) => 5,
        ProfileError::InvalidTransition { .. } => 6,
        ProfileError::InvalidInput(..) => 7,
        ProfileError::NonPositiveAmount => 8,
        ProfileError::InsufficientQuantity { .. } => 9,
        ProfileError::HardDeleteForbidden => 10,
        ProfileError::FloatForbidden => 11,
        ProfileError::OwnershipMismatch { .. } => 12,
        ProfileError::NotTransferable => 13,
    }
}
