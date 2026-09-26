use crate::{
    EntitlementError, LEASE_COMMITMENT_LEN, LeaseTimeInputV1, MAX_ARTIFACT_SIZE, MAX_GRANT_LEN,
    MAX_GRANTS, MAX_ID_LEN, MAX_KID_LEN, MAX_PRODUCT_ID_LEN, TimeAcceptance, TimePolicy,
    TrustBundle, TrustPlane, TrustedTimeError, TrustedTimeStateV1, bounded_nonempty,
    evaluate_time_bound_right_strict,
};
use coset::{
    ContentType, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable, iana,
};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use thiserror::Error;

pub const OFFLINE_BORROW_CONTENT_TYPE: &str =
    "application/vnd.chaptera.offline-borrow+cbor";
pub const MAX_BORROW_COMMAND_RECEIPTS: usize = 256;
const BORROW_SCOPE_DOMAIN: &[u8] = b"Chaptera.OfflineBorrow.TrustedTimeScope.v1\0";
const ISSUE_COMMAND_DOMAIN: &[u8] = b"Chaptera.OfflineBorrow.ISSUE.v1\0";
const EXTEND_COMMAND_DOMAIN: &[u8] = b"Chaptera.OfflineBorrow.EXTEND.v1\0";
const END_COMMAND_DOMAIN: &[u8] = b"Chaptera.OfflineBorrow.END.v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineBorrowLeaseV1 {
    schema_version: u16,
    entitlement_id: String,
    seat_id: String,
    borrow_lineage_id: String,
    lease_id: String,
    lease_sequence: u64,
    issued_at: i64,
    not_before: i64,
    new_paid_work_until: i64,
    hard_valid_until: i64,
    parent_entitlement_revision: u64,
    parent_hard_valid_until: Option<i64>,
    product_id: String,
    grants: Vec<String>,
}

impl OfflineBorrowLeaseV1 {
    pub fn entitlement_id(&self) -> &str {
        &self.entitlement_id
    }

    pub fn seat_id(&self) -> &str {
        &self.seat_id
    }

    pub fn borrow_lineage_id(&self) -> &str {
        &self.borrow_lineage_id
    }

    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub fn lease_sequence(&self) -> u64 {
        self.lease_sequence
    }

    pub fn issued_at(&self) -> i64 {
        self.issued_at
    }

    pub fn not_before(&self) -> i64 {
        self.not_before
    }

    pub fn new_paid_work_until(&self) -> i64 {
        self.new_paid_work_until
    }

    pub fn hard_valid_until(&self) -> i64 {
        self.hard_valid_until
    }

    pub fn parent_entitlement_revision(&self) -> u64 {
        self.parent_entitlement_revision
    }

    pub fn parent_hard_valid_until(&self) -> Option<i64> {
        self.parent_hard_valid_until
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn grants(&self) -> &[String] {
        &self.grants
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOfflineBorrowLease {
    lease: OfflineBorrowLeaseV1,
    lease_commitment: [u8; LEASE_COMMITMENT_LEN],
    signer_kid: Vec<u8>,
}

impl VerifiedOfflineBorrowLease {
    pub fn lease(&self) -> &OfflineBorrowLeaseV1 {
        &self.lease
    }

    pub fn lease_commitment(&self) -> &[u8; LEASE_COMMITMENT_LEN] {
        &self.lease_commitment
    }

    pub fn signer_kid(&self) -> &[u8] {
        &self.signer_kid
    }

    pub fn trusted_time_scope_id(&self) -> [u8; 32] {
        borrow_scope_id(self.lease.entitlement_id(), self.lease.seat_id())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OfflineBorrowVerifyError {
    #[error("offline-borrow artifact too large")]
    ArtifactTooLarge,
    #[error("offline-borrow artifact malformed")]
    MalformedArtifact,
    #[error("offline-borrow schema unsupported")]
    UnsupportedSchema,
    #[error("offline-borrow algorithm unsupported")]
    UnsupportedAlgorithm,
    #[error("offline-borrow unprotected headers are forbidden")]
    UnexpectedUnprotectedHeaders,
    #[error("offline-borrow content type mismatch")]
    UnexpectedContentType,
    #[error("offline-borrow signer key id malformed")]
    MalformedKeyId,
    #[error("offline-borrow signature invalid")]
    SignatureInvalid,
    #[error("offline-borrow payload violates V1 policy")]
    PayloadPolicyViolation,
    #[error("offline-borrow product mismatch")]
    ProductMismatch,
    #[error("offline-borrow trust failure: {0}")]
    Trust(EntitlementError),
}

fn decode_exact<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
) -> Result<T, OfflineBorrowVerifyError> {
    let mut cursor = Cursor::new(bytes);
    let value = ciborium::de::from_reader(&mut cursor)
        .map_err(|_| OfflineBorrowVerifyError::MalformedArtifact)?;
    if cursor.position() != bytes.len() as u64 {
        return Err(OfflineBorrowVerifyError::MalformedArtifact);
    }
    Ok(value)
}

fn validate_lease_shape(lease: &OfflineBorrowLeaseV1) -> Result<(), OfflineBorrowVerifyError> {
    if lease.schema_version != 1 {
        return Err(OfflineBorrowVerifyError::UnsupportedSchema);
    }

    if !bounded_nonempty(&lease.entitlement_id, MAX_ID_LEN)
        || !bounded_nonempty(&lease.seat_id, MAX_ID_LEN)
        || !bounded_nonempty(&lease.borrow_lineage_id, MAX_ID_LEN)
        || !bounded_nonempty(&lease.lease_id, MAX_ID_LEN)
        || !bounded_nonempty(&lease.product_id, MAX_PRODUCT_ID_LEN)
        || lease.lease_sequence == 0
        || lease.parent_entitlement_revision == 0
        || lease.issued_at < 0
        || lease.not_before < lease.issued_at
        || lease.new_paid_work_until <= lease.not_before
        || lease.hard_valid_until <= lease.new_paid_work_until
        || lease
            .parent_hard_valid_until
            .is_some_and(|parent| lease.hard_valid_until > parent)
        || lease.grants.is_empty()
        || lease.grants.len() > MAX_GRANTS
        || lease
            .grants
            .iter()
            .any(|grant| !bounded_nonempty(grant, MAX_GRANT_LEN))
    {
        return Err(OfflineBorrowVerifyError::PayloadPolicyViolation);
    }

    for (index, grant) in lease.grants.iter().enumerate() {
        if lease.grants[..index].iter().any(|seen| seen == grant) {
            return Err(OfflineBorrowVerifyError::PayloadPolicyViolation);
        }
    }

    Ok(())
}

pub fn verify_offline_borrow_lease(
    artifact: &[u8],
    trust: &TrustBundle,
    expected_product_id: &str,
) -> Result<VerifiedOfflineBorrowLease, OfflineBorrowVerifyError> {
    if artifact.len() > MAX_ARTIFACT_SIZE {
        return Err(OfflineBorrowVerifyError::ArtifactTooLarge);
    }
    if trust.plane() != TrustPlane::Entitlement {
        return Err(OfflineBorrowVerifyError::Trust(
            EntitlementError::InvalidTrustBundle,
        ));
    }

    let sign1 = CoseSign1::from_tagged_slice(artifact)
        .map_err(|_| OfflineBorrowVerifyError::MalformedArtifact)?;
    if !sign1.unprotected.is_empty() {
        return Err(OfflineBorrowVerifyError::UnexpectedUnprotectedHeaders);
    }

    let protected = &sign1.protected.header;
    if !protected.crit.is_empty()
        || !protected.iv.is_empty()
        || !protected.partial_iv.is_empty()
        || !protected.counter_signatures.is_empty()
        || !protected.rest.is_empty()
    {
        return Err(OfflineBorrowVerifyError::MalformedArtifact);
    }

    match protected.alg.as_ref() {
        Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::ESP256)) => {}
        _ => return Err(OfflineBorrowVerifyError::UnsupportedAlgorithm),
    }
    match &protected.content_type {
        Some(ContentType::Text(value)) if value == OFFLINE_BORROW_CONTENT_TYPE => {}
        _ => return Err(OfflineBorrowVerifyError::UnexpectedContentType),
    }
    if protected.key_id.is_empty() || protected.key_id.len() > MAX_KID_LEN {
        return Err(OfflineBorrowVerifyError::MalformedKeyId);
    }

    let signer = trust
        .resolve(&protected.key_id)
        .map_err(OfflineBorrowVerifyError::Trust)?;
    let verifying_key = VerifyingKey::from_sec1_bytes(signer.public_key_sec1())
        .map_err(|_| OfflineBorrowVerifyError::Trust(EntitlementError::InvalidTrustBundle))?;

    if sign1.signature.len() != 64 {
        return Err(OfflineBorrowVerifyError::SignatureInvalid);
    }
    sign1
        .verify_signature(&[], |sig_bytes, tbs| {
            let signature = Signature::from_slice(sig_bytes).map_err(|_| ())?;
            verifying_key.verify(tbs, &signature).map_err(|_| ())
        })
        .map_err(|_| OfflineBorrowVerifyError::SignatureInvalid)?;

    let payload_bytes = sign1
        .payload
        .as_deref()
        .ok_or(OfflineBorrowVerifyError::MalformedArtifact)?;
    let lease: OfflineBorrowLeaseV1 = decode_exact(payload_bytes)?;
    validate_lease_shape(&lease)?;
    if lease.product_id != expected_product_id {
        return Err(OfflineBorrowVerifyError::ProductMismatch);
    }

    Ok(VerifiedOfflineBorrowLease {
        lease,
        lease_commitment: Sha256::digest(payload_bytes).into(),
        signer_kid: protected.key_id.clone(),
    })
}

fn borrow_scope_id(entitlement_id: &str, seat_id: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(BORROW_SCOPE_DOMAIN);
    hash.update((entitlement_id.len() as u64).to_be_bytes());
    hash.update(entitlement_id.as_bytes());
    hash.update((seat_id.len() as u64).to_be_bytes());
    hash.update(seat_id.as_bytes());
    hash.finalize().into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfflineBorrowDecision {
    WorkAllowed {
        next_state: TrustedTimeStateV1,
    },
    DrainOnly {
        until: i64,
        next_state: TrustedTimeStateV1,
    },
    Expired,
    NotYetValid,
    TimeUncertain,
    Invalid(TrustedTimeError),
}

pub fn evaluate_offline_borrow(
    state: &TrustedTimeStateV1,
    now: i64,
    authenticated_server_time: Option<i64>,
    lease: &VerifiedOfflineBorrowLease,
    policy: TimePolicy,
) -> OfflineBorrowDecision {
    let scope = lease.trusted_time_scope_id();
    let signed = lease.lease();
    let input = LeaseTimeInputV1 {
        lease_id: signed.lease_id(),
        lease_sequence: signed.lease_sequence(),
        issued_at: signed.issued_at(),
        not_before: signed.not_before(),
        valid_until: signed.new_paid_work_until(),
        offline_grace_until: Some(signed.hard_valid_until()),
        authenticated_server_time,
        lease_commitment: *lease.lease_commitment(),
    };

    match evaluate_time_bound_right_strict(state, &scope, now, &input, policy) {
        Ok(TimeAcceptance::Valid { next_state }) => {
            OfflineBorrowDecision::WorkAllowed { next_state }
        }
        Ok(TimeAcceptance::Grace { until, next_state }) => {
            OfflineBorrowDecision::DrainOnly { until, next_state }
        }
        Err(TrustedTimeError::Expired) => OfflineBorrowDecision::Expired,
        Err(TrustedTimeError::NotYetValid) => OfflineBorrowDecision::NotYetValid,
        Err(TrustedTimeError::ClockRollback | TrustedTimeError::TimeUncertain) => {
            OfflineBorrowDecision::TimeUncertain
        }
        Err(error) => OfflineBorrowDecision::Invalid(error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowSeatStatus {
    Available,
    OnlineCheckedOut {
        session_id: String,
        lease_until: i64,
    },
    OfflineReserved {
        borrow_lineage_id: String,
        lease_id: String,
        lease_sequence: u64,
        new_paid_work_until: i64,
        hard_valid_until: i64,
        end_reported: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SeatState {
    Available,
    OnlineCheckedOut {
        session_id: String,
        lease_until: i64,
    },
    OfflineReserved {
        latest_lease: OfflineBorrowLeaseV1,
        end_reported: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandReceipt {
    command_id: String,
    commitment: [u8; 32],
    outcome: BorrowAuthorityOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineBorrowAuthorityState {
    seat_id: String,
    generation: u64,
    next_sequence: u64,
    state: SeatState,
    receipts: Vec<CommandReceipt>,
}

impl OfflineBorrowAuthorityState {
    pub fn available(
        seat_id: impl Into<String>,
        next_sequence: u64,
    ) -> Result<Self, BorrowAuthorityError> {
        let seat_id = seat_id.into();
        validate_authority_id(&seat_id)?;
        if next_sequence == 0 {
            return Err(BorrowAuthorityError::SequenceExhausted);
        }
        Ok(Self {
            seat_id,
            generation: 0,
            next_sequence,
            state: SeatState::Available,
            receipts: Vec::new(),
        })
    }

    pub fn online_checked_out(
        seat_id: impl Into<String>,
        next_sequence: u64,
        session_id: impl Into<String>,
        lease_until: i64,
    ) -> Result<Self, BorrowAuthorityError> {
        let seat_id = seat_id.into();
        let session_id = session_id.into();
        validate_authority_id(&seat_id)?;
        validate_authority_id(&session_id)?;
        if next_sequence == 0 {
            return Err(BorrowAuthorityError::SequenceExhausted);
        }
        Ok(Self {
            seat_id,
            generation: 0,
            next_sequence,
            state: SeatState::OnlineCheckedOut {
                session_id,
                lease_until,
            },
            receipts: Vec::new(),
        })
    }

    pub fn seat_id(&self) -> &str {
        &self.seat_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub fn status(&self, authoritative_now: i64) -> BorrowSeatStatus {
        match normalized_seat_state(&self.state, authoritative_now) {
            SeatState::Available => BorrowSeatStatus::Available,
            SeatState::OnlineCheckedOut {
                session_id,
                lease_until,
            } => BorrowSeatStatus::OnlineCheckedOut {
                session_id,
                lease_until,
            },
            SeatState::OfflineReserved {
                latest_lease,
                end_reported,
            } => BorrowSeatStatus::OfflineReserved {
                borrow_lineage_id: latest_lease.borrow_lineage_id.clone(),
                lease_id: latest_lease.lease_id.clone(),
                lease_sequence: latest_lease.lease_sequence,
                new_paid_work_until: latest_lease.new_paid_work_until,
                hard_valid_until: latest_lease.hard_valid_until,
                end_reported,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentEntitlementConstraint {
    pub revision: u64,
    pub hard_valid_until: Option<i64>,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueBorrowCommand {
    pub command_id: String,
    pub expected_generation: u64,
    pub entitlement_id: String,
    pub borrow_lineage_id: String,
    pub lease_id: String,
    pub requested_new_paid_work_until: i64,
    pub drain_seconds: i64,
    pub product_id: String,
    pub grants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtendBorrowCommand {
    pub command_id: String,
    pub expected_generation: u64,
    pub expected_lease_id: String,
    pub expected_sequence: u64,
    pub new_lease_id: String,
    pub requested_new_paid_work_until: i64,
    pub drain_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndOfflineModeCommand {
    pub command_id: String,
    pub expected_generation: u64,
    pub borrow_lineage_id: String,
    pub expected_lease_id: String,
    pub expected_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowAuthorityOutcome {
    Issued {
        lease: OfflineBorrowLeaseV1,
    },
    Extended {
        lease: OfflineBorrowLeaseV1,
    },
    EndRecorded {
        hard_valid_until: i64,
    },
    AlreadyEnded {
        hard_valid_until: i64,
    },
    NeedsConfirmation {
        max_new_paid_work_until: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowAuthorityTransition {
    outcome: BorrowAuthorityOutcome,
    next_state: OfflineBorrowAuthorityState,
}

impl BorrowAuthorityTransition {
    pub fn outcome(&self) -> &BorrowAuthorityOutcome {
        &self.outcome
    }

    pub fn next_state(&self) -> &OfflineBorrowAuthorityState {
        &self.next_state
    }

    pub fn into_next_state(self) -> OfflineBorrowAuthorityState {
        self.next_state
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BorrowAuthorityError {
    #[error("borrow authority identifier violates V1 bounds")]
    InvalidIdentifier,
    #[error("borrow authority command malformed")]
    InvalidCommand,
    #[error("parent entitlement is revoked or expired")]
    ParentUnavailable,
    #[error("parent entitlement revision moved backward")]
    ParentRevisionStale,
    #[error("requested borrow window cannot fit before parent hard boundary")]
    ParentWindowTooShort,
    #[error("seat is unavailable")]
    SeatUnavailable,
    #[error("borrow lineage or latest lease is stale")]
    StaleLease,
    #[error("offline mode was already ended; extension is forbidden")]
    EndAlreadyReported,
    #[error("authority generation is stale")]
    StaleAuthorityGeneration,
    #[error("command id was reused with different facts")]
    IdempotencyConflict,
    #[error("idempotency receipt history is full")]
    ReceiptHistoryFull,
    #[error("seat-scoped sequence is exhausted")]
    SequenceExhausted,
    #[error("authority generation is exhausted")]
    GenerationExhausted,
}

fn validate_authority_id(value: &str) -> Result<(), BorrowAuthorityError> {
    if bounded_nonempty(value, MAX_ID_LEN) {
        Ok(())
    } else {
        Err(BorrowAuthorityError::InvalidIdentifier)
    }
}

fn validate_grants(grants: &[String]) -> Result<(), BorrowAuthorityError> {
    if grants.is_empty()
        || grants.len() > MAX_GRANTS
        || grants
            .iter()
            .any(|grant| !bounded_nonempty(grant, MAX_GRANT_LEN))
    {
        return Err(BorrowAuthorityError::InvalidCommand);
    }
    for (index, grant) in grants.iter().enumerate() {
        if grants[..index].iter().any(|seen| seen == grant) {
            return Err(BorrowAuthorityError::InvalidCommand);
        }
    }
    Ok(())
}

fn normalized_seat_state(state: &SeatState, authoritative_now: i64) -> SeatState {
    match state {
        SeatState::OnlineCheckedOut { lease_until, .. }
            if authoritative_now >= *lease_until =>
        {
            SeatState::Available
        }
        SeatState::OfflineReserved { latest_lease, .. }
            if authoritative_now >= latest_lease.hard_valid_until =>
        {
            SeatState::Available
        }
        other => other.clone(),
    }
}

pub fn normalize_borrow_expiry(
    state: &OfflineBorrowAuthorityState,
    authoritative_now: i64,
) -> Result<OfflineBorrowAuthorityState, BorrowAuthorityError> {
    let normalized = normalized_seat_state(&state.state, authoritative_now);
    if normalized == state.state {
        return Ok(state.clone());
    }
    let mut next = state.clone();
    next.state = normalized;
    next.generation = next
        .generation
        .checked_add(1)
        .ok_or(BorrowAuthorityError::GenerationExhausted)?;
    Ok(next)
}

fn command_commitment<T: Serialize>(
    domain: &[u8],
    command: &T,
) -> Result<[u8; 32], BorrowAuthorityError> {
    let mut encoded = Vec::new();
    ciborium::ser::into_writer(command, &mut encoded)
        .map_err(|_| BorrowAuthorityError::InvalidCommand)?;
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(encoded);
    Ok(hash.finalize().into())
}

fn replay_if_known(
    state: &OfflineBorrowAuthorityState,
    command_id: &str,
    commitment: [u8; 32],
) -> Result<Option<BorrowAuthorityTransition>, BorrowAuthorityError> {
    if let Some(receipt) = state
        .receipts
        .iter()
        .find(|receipt| receipt.command_id == command_id)
    {
        if receipt.commitment != commitment {
            return Err(BorrowAuthorityError::IdempotencyConflict);
        }
        return Ok(Some(BorrowAuthorityTransition {
            outcome: receipt.outcome.clone(),
            next_state: state.clone(),
        }));
    }
    Ok(None)
}

fn record_receipt(
    state: &mut OfflineBorrowAuthorityState,
    command_id: String,
    commitment: [u8; 32],
    outcome: BorrowAuthorityOutcome,
) -> Result<(), BorrowAuthorityError> {
    if state.receipts.len() >= MAX_BORROW_COMMAND_RECEIPTS {
        return Err(BorrowAuthorityError::ReceiptHistoryFull);
    }
    state.receipts.push(CommandReceipt {
        command_id,
        commitment,
        outcome,
    });
    Ok(())
}

fn next_generation(
    state: &OfflineBorrowAuthorityState,
) -> Result<u64, BorrowAuthorityError> {
    state
        .generation
        .checked_add(1)
        .ok_or(BorrowAuthorityError::GenerationExhausted)
}

fn next_sequence(sequence: u64) -> Result<u64, BorrowAuthorityError> {
    sequence
        .checked_add(1)
        .ok_or(BorrowAuthorityError::SequenceExhausted)
}

fn maximum_work_cutoff(
    authoritative_now: i64,
    drain_seconds: i64,
    parent: ParentEntitlementConstraint,
) -> Result<Option<i64>, BorrowAuthorityError> {
    if parent.revoked || parent.revision == 0 {
        return Err(BorrowAuthorityError::ParentUnavailable);
    }
    let Some(parent_hard) = parent.hard_valid_until else {
        return Ok(None);
    };
    if authoritative_now >= parent_hard {
        return Err(BorrowAuthorityError::ParentUnavailable);
    }
    let max_work = parent_hard
        .checked_sub(drain_seconds)
        .ok_or(BorrowAuthorityError::ParentWindowTooShort)?;
    if max_work <= authoritative_now {
        return Err(BorrowAuthorityError::ParentWindowTooShort);
    }
    Ok(Some(max_work))
}

pub fn issue_borrow(
    state: &OfflineBorrowAuthorityState,
    command: &IssueBorrowCommand,
    parent: ParentEntitlementConstraint,
    authoritative_now: i64,
) -> Result<BorrowAuthorityTransition, BorrowAuthorityError> {
    validate_authority_id(&command.command_id)?;
    validate_authority_id(&command.entitlement_id)?;
    validate_authority_id(&command.borrow_lineage_id)?;
    validate_authority_id(&command.lease_id)?;
    if !bounded_nonempty(&command.product_id, MAX_PRODUCT_ID_LEN)
        || command.drain_seconds <= 0
        || command.requested_new_paid_work_until <= authoritative_now
    {
        return Err(BorrowAuthorityError::InvalidCommand);
    }
    validate_grants(&command.grants)?;

    let commitment = command_commitment(ISSUE_COMMAND_DOMAIN, command)?;
    if let Some(replay) = replay_if_known(state, &command.command_id, commitment)? {
        return Ok(replay);
    }
    if command.expected_generation != state.generation {
        return Err(BorrowAuthorityError::StaleAuthorityGeneration);
    }

    let normalized = normalized_seat_state(&state.state, authoritative_now);
    if normalized != SeatState::Available {
        return Err(BorrowAuthorityError::SeatUnavailable);
    }

    let hard_valid_until = command
        .requested_new_paid_work_until
        .checked_add(command.drain_seconds)
        .ok_or(BorrowAuthorityError::InvalidCommand)?;
    let max_work = maximum_work_cutoff(authoritative_now, command.drain_seconds, parent)?;
    if let Some(max_work) = max_work
        && command.requested_new_paid_work_until > max_work
    {
        return Ok(BorrowAuthorityTransition {
            outcome: BorrowAuthorityOutcome::NeedsConfirmation {
                max_new_paid_work_until: max_work,
            },
            next_state: state.clone(),
        });
    }

    let sequence = state.next_sequence;
    let successor_sequence = next_sequence(sequence)?;
    let lease = OfflineBorrowLeaseV1 {
        schema_version: 1,
        entitlement_id: command.entitlement_id.clone(),
        seat_id: state.seat_id.clone(),
        borrow_lineage_id: command.borrow_lineage_id.clone(),
        lease_id: command.lease_id.clone(),
        lease_sequence: sequence,
        issued_at: authoritative_now,
        not_before: authoritative_now,
        new_paid_work_until: command.requested_new_paid_work_until,
        hard_valid_until,
        parent_entitlement_revision: parent.revision,
        parent_hard_valid_until: parent.hard_valid_until,
        product_id: command.product_id.clone(),
        grants: command.grants.clone(),
    };
    validate_lease_shape(&lease).map_err(|_| BorrowAuthorityError::InvalidCommand)?;

    let outcome = BorrowAuthorityOutcome::Issued {
        lease: lease.clone(),
    };
    let mut next = state.clone();
    next.generation = next_generation(state)?;
    next.next_sequence = successor_sequence;
    next.state = SeatState::OfflineReserved {
        latest_lease: lease,
        end_reported: false,
    };
    record_receipt(
        &mut next,
        command.command_id.clone(),
        commitment,
        outcome.clone(),
    )?;

    Ok(BorrowAuthorityTransition {
        outcome,
        next_state: next,
    })
}

pub fn extend_borrow(
    state: &OfflineBorrowAuthorityState,
    command: &ExtendBorrowCommand,
    parent: ParentEntitlementConstraint,
    authoritative_now: i64,
) -> Result<BorrowAuthorityTransition, BorrowAuthorityError> {
    validate_authority_id(&command.command_id)?;
    validate_authority_id(&command.expected_lease_id)?;
    validate_authority_id(&command.new_lease_id)?;
    if command.expected_sequence == 0
        || command.drain_seconds <= 0
        || command.requested_new_paid_work_until <= authoritative_now
    {
        return Err(BorrowAuthorityError::InvalidCommand);
    }

    let commitment = command_commitment(EXTEND_COMMAND_DOMAIN, command)?;
    if let Some(replay) = replay_if_known(state, &command.command_id, commitment)? {
        return Ok(replay);
    }
    if command.expected_generation != state.generation {
        return Err(BorrowAuthorityError::StaleAuthorityGeneration);
    }

    let normalized = normalized_seat_state(&state.state, authoritative_now);
    let SeatState::OfflineReserved {
        latest_lease,
        end_reported,
    } = normalized
    else {
        return Err(BorrowAuthorityError::SeatUnavailable);
    };
    if end_reported {
        return Err(BorrowAuthorityError::EndAlreadyReported);
    }
    if latest_lease.lease_id != command.expected_lease_id
        || latest_lease.lease_sequence != command.expected_sequence
    {
        return Err(BorrowAuthorityError::StaleLease);
    }
    if parent.revoked {
        return Err(BorrowAuthorityError::ParentUnavailable);
    }
    if parent.revision < latest_lease.parent_entitlement_revision {
        return Err(BorrowAuthorityError::ParentRevisionStale);
    }
    if command.requested_new_paid_work_until <= latest_lease.new_paid_work_until {
        return Err(BorrowAuthorityError::InvalidCommand);
    }

    let hard_valid_until = command
        .requested_new_paid_work_until
        .checked_add(command.drain_seconds)
        .ok_or(BorrowAuthorityError::InvalidCommand)?;
    if hard_valid_until <= latest_lease.hard_valid_until {
        return Err(BorrowAuthorityError::InvalidCommand);
    }

    let max_work = maximum_work_cutoff(authoritative_now, command.drain_seconds, parent)?;
    if let Some(max_work) = max_work {
        if max_work <= latest_lease.new_paid_work_until {
            return Err(BorrowAuthorityError::ParentWindowTooShort);
        }
        if command.requested_new_paid_work_until > max_work {
            return Ok(BorrowAuthorityTransition {
                outcome: BorrowAuthorityOutcome::NeedsConfirmation {
                    max_new_paid_work_until: max_work,
                },
                next_state: state.clone(),
            });
        }
    }

    let sequence = state.next_sequence;
    if sequence <= latest_lease.lease_sequence {
        return Err(BorrowAuthorityError::SequenceExhausted);
    }
    let successor_sequence = next_sequence(sequence)?;
    let successor = OfflineBorrowLeaseV1 {
        schema_version: 1,
        entitlement_id: latest_lease.entitlement_id.clone(),
        seat_id: latest_lease.seat_id.clone(),
        borrow_lineage_id: latest_lease.borrow_lineage_id.clone(),
        lease_id: command.new_lease_id.clone(),
        lease_sequence: sequence,
        issued_at: authoritative_now,
        not_before: authoritative_now,
        new_paid_work_until: command.requested_new_paid_work_until,
        hard_valid_until,
        parent_entitlement_revision: parent.revision,
        parent_hard_valid_until: parent.hard_valid_until,
        product_id: latest_lease.product_id.clone(),
        grants: latest_lease.grants.clone(),
    };
    validate_lease_shape(&successor).map_err(|_| BorrowAuthorityError::InvalidCommand)?;

    let outcome = BorrowAuthorityOutcome::Extended {
        lease: successor.clone(),
    };
    let mut next = state.clone();
    next.generation = next_generation(state)?;
    next.next_sequence = successor_sequence;
    next.state = SeatState::OfflineReserved {
        latest_lease: successor,
        end_reported: false,
    };
    record_receipt(
        &mut next,
        command.command_id.clone(),
        commitment,
        outcome.clone(),
    )?;

    Ok(BorrowAuthorityTransition {
        outcome,
        next_state: next,
    })
}

pub fn end_offline_mode(
    state: &OfflineBorrowAuthorityState,
    command: &EndOfflineModeCommand,
    authoritative_now: i64,
) -> Result<BorrowAuthorityTransition, BorrowAuthorityError> {
    validate_authority_id(&command.command_id)?;
    validate_authority_id(&command.borrow_lineage_id)?;
    validate_authority_id(&command.expected_lease_id)?;
    if command.expected_sequence == 0 {
        return Err(BorrowAuthorityError::InvalidCommand);
    }

    let commitment = command_commitment(END_COMMAND_DOMAIN, command)?;
    if let Some(replay) = replay_if_known(state, &command.command_id, commitment)? {
        return Ok(replay);
    }
    if command.expected_generation != state.generation {
        return Err(BorrowAuthorityError::StaleAuthorityGeneration);
    }

    let normalized = normalized_seat_state(&state.state, authoritative_now);
    let SeatState::OfflineReserved {
        latest_lease,
        end_reported,
    } = normalized
    else {
        return Err(BorrowAuthorityError::StaleLease);
    };
    if latest_lease.borrow_lineage_id != command.borrow_lineage_id
        || latest_lease.lease_id != command.expected_lease_id
        || latest_lease.lease_sequence != command.expected_sequence
    {
        return Err(BorrowAuthorityError::StaleLease);
    }

    if end_reported {
        return Ok(BorrowAuthorityTransition {
            outcome: BorrowAuthorityOutcome::AlreadyEnded {
                hard_valid_until: latest_lease.hard_valid_until,
            },
            next_state: state.clone(),
        });
    }

    let outcome = BorrowAuthorityOutcome::EndRecorded {
        hard_valid_until: latest_lease.hard_valid_until,
    };
    let mut next = state.clone();
    next.generation = next_generation(state)?;
    next.state = SeatState::OfflineReserved {
        latest_lease,
        end_reported: true,
    };
    record_receipt(
        &mut next,
        command.command_id.clone(),
        commitment,
        outcome.clone(),
    )?;

    Ok(BorrowAuthorityTransition {
        outcome,
        next_state: next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TrustedSigner, TrustBundle, TrustPlane};
    use ciborium::value::Value;
    use coset::{CoseSign1Builder, HeaderBuilder};
    use p256::ecdsa::{SigningKey, signature::Signer};

    const PRODUCT: &str = "chaptera.rescue.technician";
    const SEAT: &str = "seat-2";
    const ENTITLEMENT: &str = "ent-tech-1";
    const T0: i64 = 1_900_000_000;
    const WORK: i64 = T0 + 24 * 60 * 60;
    const HARD: i64 = WORK + 10 * 60;
    const TEST_KID: &[u8] = b"test:borrow:k1";

    fn signing_key() -> SigningKey {
        SigningKey::from_slice(&[0x42; 32]).expect("borrow test signer")
    }

    fn test_trust() -> TrustBundle {
        let public = signing_key()
            .verifying_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec();
        TrustBundle::testing(
            TrustPlane::Entitlement,
            vec![TrustedSigner::new(TEST_KID.to_vec(), public)],
        )
    }

    fn parent(hard: Option<i64>) -> ParentEntitlementConstraint {
        ParentEntitlementConstraint {
            revision: 7,
            hard_valid_until: hard,
            revoked: false,
        }
    }

    fn issue_command(command_id: &str, expected_generation: u64) -> IssueBorrowCommand {
        IssueBorrowCommand {
            command_id: command_id.into(),
            expected_generation,
            entitlement_id: ENTITLEMENT.into(),
            borrow_lineage_id: "borrow-lineage-1".into(),
            lease_id: "borrow-lease-1".into(),
            requested_new_paid_work_until: WORK,
            drain_seconds: 10 * 60,
            product_id: PRODUCT.into(),
            grants: vec!["recover".into(), "export".into()],
        }
    }

    fn issued_state() -> OfflineBorrowAuthorityState {
        issue_borrow(
            &OfflineBorrowAuthorityState::available(SEAT, 1).unwrap(),
            &issue_command("issue-1", 0),
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0,
        )
        .unwrap()
        .into_next_state()
    }

    fn latest_lease(state: &OfflineBorrowAuthorityState) -> OfflineBorrowLeaseV1 {
        match &state.state {
            SeatState::OfflineReserved { latest_lease, .. } => latest_lease.clone(),
            _ => panic!("expected offline reservation"),
        }
    }

    fn sign_lease(lease: &OfflineBorrowLeaseV1) -> Vec<u8> {
        let mut payload = Vec::new();
        ciborium::ser::into_writer(lease, &mut payload).expect("borrow payload");
        let protected = HeaderBuilder::new()
            .algorithm(iana::Algorithm::ESP256)
            .key_id(TEST_KID.to_vec())
            .content_type(OFFLINE_BORROW_CONTENT_TYPE.to_owned())
            .build();
        CoseSign1Builder::new()
            .protected(protected)
            .payload(payload)
            .create_signature(&[], |tbs| {
                let signature: Signature = signing_key().sign(tbs);
                signature.to_bytes().to_vec()
            })
            .build()
            .to_tagged_vec()
            .expect("borrow COSE")
    }

    fn verified(lease: &OfflineBorrowLeaseV1) -> VerifiedOfflineBorrowLease {
        verify_offline_borrow_lease(&sign_lease(lease), &test_trust(), PRODUCT)
            .expect("verified borrow")
    }

    fn time_policy() -> TimePolicy {
        TimePolicy {
            skew_seconds: 300,
            forward_jump_limit_seconds: Some(90 * 24 * 60 * 60),
        }
    }

    #[test]
    fn strict_signed_borrow_round_trip_and_product_policy() {
        let lease = latest_lease(&issued_state());
        let artifact = sign_lease(&lease);
        let verified =
            verify_offline_borrow_lease(&artifact, &test_trust(), PRODUCT).expect("valid");
        assert_eq!(verified.lease().seat_id(), SEAT);
        assert_eq!(verified.lease().lease_sequence(), 1);
        assert_eq!(verified.signer_kid(), TEST_KID);

        let wrong =
            verify_offline_borrow_lease(&artifact, &test_trust(), "chaptera.editor").unwrap_err();
        assert_eq!(wrong, OfflineBorrowVerifyError::ProductMismatch);
    }

    #[test]
    fn corrupted_bearer_signature_is_invalid() {
        let lease = latest_lease(&issued_state());
        let artifact = sign_lease(&lease);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).expect("COSE");
        sign1.signature[0] ^= 0x80;
        let corrupted = sign1.to_tagged_vec().expect("COSE");
        let err = verify_offline_borrow_lease(&corrupted, &test_trust(), PRODUCT).unwrap_err();
        assert_eq!(err, OfflineBorrowVerifyError::SignatureInvalid);
    }

    #[test]
    fn unknown_signed_payload_field_is_rejected() {
        let lease = latest_lease(&issued_state());
        let mut value = Value::serialized(&lease).expect("payload value");
        let Value::Map(entries) = &mut value else {
            panic!("lease must be a map");
        };
        entries.push((
            Value::Text("customer_filename".into()),
            Value::Text("must-not-enter-licensing-proof".into()),
        ));
        let mut payload = Vec::new();
        ciborium::ser::into_writer(&value, &mut payload).expect("payload");

        let protected = HeaderBuilder::new()
            .algorithm(iana::Algorithm::ESP256)
            .key_id(TEST_KID.to_vec())
            .content_type(OFFLINE_BORROW_CONTENT_TYPE.to_owned())
            .build();
        let artifact = CoseSign1Builder::new()
            .protected(protected)
            .payload(payload)
            .create_signature(&[], |tbs| {
                let signature: Signature = signing_key().sign(tbs);
                signature.to_bytes().to_vec()
            })
            .build()
            .to_tagged_vec()
            .expect("COSE");

        let err = verify_offline_borrow_lease(&artifact, &test_trust(), PRODUCT).unwrap_err();
        assert_eq!(err, OfflineBorrowVerifyError::MalformedArtifact);
    }

    #[test]
    fn bearer_clone_has_same_strict_work_drain_and_hard_cutoffs_in_isolated_runtimes() {
        let lease = verified(&latest_lease(&issued_state()));
        let scope = lease.trusted_time_scope_id();
        let state_a = TrustedTimeStateV1::fresh(scope);
        let state_b = TrustedTimeStateV1::fresh(scope);

        for state in [&state_a, &state_b] {
            assert!(matches!(
                evaluate_offline_borrow(state, WORK - 1, None, &lease, time_policy()),
                OfflineBorrowDecision::WorkAllowed { .. }
            ));
            assert!(matches!(
                evaluate_offline_borrow(state, WORK, None, &lease, time_policy()),
                OfflineBorrowDecision::DrainOnly { .. }
            ));
            assert!(matches!(
                evaluate_offline_borrow(state, HARD - 1, None, &lease, time_policy()),
                OfflineBorrowDecision::DrainOnly { .. }
            ));
            assert_eq!(
                evaluate_offline_borrow(state, HARD, None, &lease, time_policy()),
                OfflineBorrowDecision::Expired
            );
        }
    }

    #[test]
    fn issue_is_idempotent_and_generation_models_one_winner_for_one_free_seat() {
        let initial = OfflineBorrowAuthorityState::available(SEAT, 1).unwrap();
        let command = issue_command("issue-1", 0);
        let first = issue_borrow(
            &initial,
            &command,
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0,
        )
        .unwrap();
        let committed = first.next_state().clone();

        let replay = issue_borrow(
            &committed,
            &command,
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0 + 30,
        )
        .unwrap();
        assert_eq!(replay.next_state(), &committed);
        assert_eq!(replay.outcome(), first.outcome());

        let loser = issue_borrow(
            &committed,
            &issue_command("issue-racer", 0),
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0,
        )
        .unwrap_err();
        assert_eq!(loser, BorrowAuthorityError::StaleAuthorityGeneration);
    }

    #[test]
    fn end_offline_mode_never_releases_bearer_capacity_early() {
        let reserved = issued_state();
        let lease = latest_lease(&reserved);
        let end = EndOfflineModeCommand {
            command_id: "end-1".into(),
            expected_generation: reserved.generation(),
            borrow_lineage_id: lease.borrow_lineage_id().into(),
            expected_lease_id: lease.lease_id().into(),
            expected_sequence: lease.lease_sequence(),
        };
        let ended = end_offline_mode(&reserved, &end, T0 + 60)
            .unwrap()
            .into_next_state();

        assert!(matches!(
            ended.status(T0 + 60),
            BorrowSeatStatus::OfflineReserved {
                end_reported: true,
                hard_valid_until: HARD,
                ..
            }
        ));

        let retry = end_offline_mode(&ended, &end, T0 + 120).unwrap();
        assert_eq!(retry.next_state(), &ended);

        let independent = issue_borrow(
            &ended,
            &issue_command("issue-2", ended.generation()),
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0 + 120,
        )
        .unwrap_err();
        assert_eq!(independent, BorrowAuthorityError::SeatUnavailable);
    }

    #[test]
    fn extension_is_same_lineage_monotonic_and_stale_extension_cannot_mint_future_rights() {
        let state = issued_state();
        let first = latest_lease(&state);
        let extend = ExtendBorrowCommand {
            command_id: "extend-1".into(),
            expected_generation: state.generation(),
            expected_lease_id: first.lease_id().into(),
            expected_sequence: first.lease_sequence(),
            new_lease_id: "borrow-lease-2".into(),
            requested_new_paid_work_until: WORK + 24 * 60 * 60,
            drain_seconds: 10 * 60,
        };
        let transition = extend_borrow(
            &state,
            &extend,
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0 + 60,
        )
        .unwrap();
        let extended = transition.next_state().clone();
        let second = latest_lease(&extended);
        assert_eq!(second.borrow_lineage_id(), first.borrow_lineage_id());
        assert_eq!(second.lease_sequence(), 2);
        assert!(second.hard_valid_until() > first.hard_valid_until());

        let retry = extend_borrow(
            &extended,
            &extend,
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0 + 120,
        )
        .unwrap();
        assert_eq!(retry.outcome(), transition.outcome());
        assert_eq!(retry.next_state(), &extended);

        let stale = ExtendBorrowCommand {
            command_id: "extend-stale".into(),
            expected_generation: extended.generation(),
            expected_lease_id: first.lease_id().into(),
            expected_sequence: first.lease_sequence(),
            new_lease_id: "borrow-lease-stale".into(),
            requested_new_paid_work_until: WORK + 2 * 24 * 60 * 60,
            drain_seconds: 10 * 60,
        };
        assert_eq!(
            extend_borrow(
                &extended,
                &stale,
                parent(Some(T0 + 30 * 24 * 60 * 60)),
                T0 + 180,
            )
            .unwrap_err(),
            BorrowAuthorityError::StaleLease
        );
    }

    #[test]
    fn parent_boundary_requires_confirmation_instead_of_silent_shortening() {
        let initial = OfflineBorrowAuthorityState::available(SEAT, 1).unwrap();
        let mut command = issue_command("issue-parent-bound", 0);
        command.requested_new_paid_work_until = T0 + 2 * 60 * 60;
        let parent_hard = T0 + 60 * 60;

        let result = issue_borrow(
            &initial,
            &command,
            parent(Some(parent_hard)),
            T0,
        )
        .unwrap();
        assert_eq!(
            result.outcome(),
            &BorrowAuthorityOutcome::NeedsConfirmation {
                max_new_paid_work_until: parent_hard - 10 * 60,
            }
        );
        assert_eq!(result.next_state(), &initial);
    }

    #[test]
    fn expiry_allows_new_lineage_without_resetting_seat_sequence_and_late_end_is_stale() {
        let first_state = issued_state();
        let first = latest_lease(&first_state);
        let normalized = normalize_borrow_expiry(&first_state, first.hard_valid_until()).unwrap();
        assert_eq!(
            normalized.status(first.hard_valid_until()),
            BorrowSeatStatus::Available
        );
        assert_eq!(normalized.next_sequence(), 2);

        let mut second_command = issue_command("issue-2", normalized.generation());
        second_command.borrow_lineage_id = "borrow-lineage-2".into();
        second_command.lease_id = "borrow-lease-2".into();
        second_command.requested_new_paid_work_until = first.hard_valid_until() + 24 * 60 * 60;
        let second_state = issue_borrow(
            &normalized,
            &second_command,
            parent(Some(T0 + 60 * 24 * 60 * 60)),
            first.hard_valid_until(),
        )
        .unwrap()
        .into_next_state();
        let second = latest_lease(&second_state);
        assert_eq!(second.lease_sequence(), 2);
        assert_eq!(second.borrow_lineage_id(), "borrow-lineage-2");

        let late_end = EndOfflineModeCommand {
            command_id: "late-end-old".into(),
            expected_generation: second_state.generation(),
            borrow_lineage_id: first.borrow_lineage_id().into(),
            expected_lease_id: first.lease_id().into(),
            expected_sequence: first.lease_sequence(),
        };
        assert_eq!(
            end_offline_mode(&second_state, &late_end, first.hard_valid_until() + 60)
                .unwrap_err(),
            BorrowAuthorityError::StaleLease
        );
    }

    #[test]
    fn trusted_time_rollback_and_extreme_forward_jump_never_grant_paid_work() {
        let lease = verified(&latest_lease(&issued_state()));
        let scope = lease.trusted_time_scope_id();
        let fresh = TrustedTimeStateV1::fresh(scope);
        let accepted = evaluate_offline_borrow(
            &fresh,
            T0 + 3600,
            None,
            &lease,
            time_policy(),
        );
        let OfflineBorrowDecision::WorkAllowed { next_state } = accepted else {
            panic!("initial lease should work");
        };

        assert_eq!(
            evaluate_offline_borrow(
                &next_state,
                T0,
                None,
                &lease,
                time_policy(),
            ),
            OfflineBorrowDecision::TimeUncertain
        );

        assert_eq!(
            evaluate_offline_borrow(
                &next_state,
                T0 + 365 * 24 * 60 * 60,
                None,
                &lease,
                time_policy(),
            ),
            OfflineBorrowDecision::TimeUncertain
        );
    }

    #[test]
    fn old_sequence_keeps_old_boundary_after_successor_extends_future_authority() {
        let state = issued_state();
        let first_lease = latest_lease(&state);
        let extend = ExtendBorrowCommand {
            command_id: "extend-boundary".into(),
            expected_generation: state.generation(),
            expected_lease_id: first_lease.lease_id().into(),
            expected_sequence: first_lease.lease_sequence(),
            new_lease_id: "borrow-lease-2".into(),
            requested_new_paid_work_until: WORK + 24 * 60 * 60,
            drain_seconds: 10 * 60,
        };
        let extended = extend_borrow(
            &state,
            &extend,
            parent(Some(T0 + 30 * 24 * 60 * 60)),
            T0 + 60,
        )
        .unwrap()
        .into_next_state();
        let second_lease = latest_lease(&extended);

        let first = verified(&first_lease);
        let second = verified(&second_lease);
        let first_state = TrustedTimeStateV1::fresh(first.trusted_time_scope_id());
        let second_state = TrustedTimeStateV1::fresh(second.trusted_time_scope_id());

        assert_eq!(
            evaluate_offline_borrow(
                &first_state,
                first_lease.hard_valid_until(),
                None,
                &first,
                time_policy(),
            ),
            OfflineBorrowDecision::Expired
        );
        assert!(matches!(
            evaluate_offline_borrow(
                &second_state,
                first_lease.hard_valid_until(),
                None,
                &second,
                time_policy(),
            ),
            OfflineBorrowDecision::WorkAllowed { .. }
        ));
    }

    #[test]
    fn parent_revocation_blocks_future_extension_without_retroactively_revoking_bearer_bytes() {
        let state = issued_state();
        let lease = latest_lease(&state);
        let verified = verified(&lease);
        let local = TrustedTimeStateV1::fresh(verified.trusted_time_scope_id());

        assert!(matches!(
            evaluate_offline_borrow(
                &local,
                T0 + 60,
                None,
                &verified,
                time_policy(),
            ),
            OfflineBorrowDecision::WorkAllowed { .. }
        ));

        let extend = ExtendBorrowCommand {
            command_id: "extend-revoked".into(),
            expected_generation: state.generation(),
            expected_lease_id: lease.lease_id().into(),
            expected_sequence: lease.lease_sequence(),
            new_lease_id: "borrow-lease-revoked".into(),
            requested_new_paid_work_until: WORK + 24 * 60 * 60,
            drain_seconds: 10 * 60,
        };
        let revoked_parent = ParentEntitlementConstraint {
            revision: 8,
            hard_valid_until: Some(T0 + 30 * 24 * 60 * 60),
            revoked: true,
        };
        assert_eq!(
            extend_borrow(&state, &extend, revoked_parent, T0 + 60).unwrap_err(),
            BorrowAuthorityError::ParentUnavailable
        );

        assert!(matches!(
            evaluate_offline_borrow(
                &local,
                T0 + 120,
                None,
                &verified,
                time_policy(),
            ),
            OfflineBorrowDecision::WorkAllowed { .. }
        ));
    }

    #[test]
    fn extension_at_hard_cutoff_cannot_resurrect_expired_lineage() {
        let state = issued_state();
        let lease = latest_lease(&state);
        let extend = ExtendBorrowCommand {
            command_id: "extend-too-late".into(),
            expected_generation: state.generation(),
            expected_lease_id: lease.lease_id().into(),
            expected_sequence: lease.lease_sequence(),
            new_lease_id: "borrow-lease-too-late".into(),
            requested_new_paid_work_until: HARD + 24 * 60 * 60,
            drain_seconds: 10 * 60,
        };
        assert_eq!(
            extend_borrow(
                &state,
                &extend,
                parent(Some(T0 + 30 * 24 * 60 * 60)),
                HARD,
            )
            .unwrap_err(),
            BorrowAuthorityError::SeatUnavailable
        );
    }

    #[test]
    fn exact_latest_signed_replay_is_idempotent_in_trusted_time() {
        let lease = verified(&latest_lease(&issued_state()));
        let scope = lease.trusted_time_scope_id();
        let first = evaluate_offline_borrow(
            &TrustedTimeStateV1::fresh(scope),
            T0 + 60,
            None,
            &lease,
            time_policy(),
        );
        let OfflineBorrowDecision::WorkAllowed { next_state } = first else {
            panic!("first acceptance");
        };
        let replay =
            evaluate_offline_borrow(&next_state, T0 + 120, None, &lease, time_policy());
        let OfflineBorrowDecision::WorkAllowed {
            next_state: replay_state,
        } = replay
        else {
            panic!("exact replay should stay valid");
        };
        assert_eq!(replay_state.highest_lease_sequence, Some(1));
        assert_eq!(replay_state.last_lease_commitment, Some(*lease.lease_commitment()));
    }

    #[test]
    fn same_sequence_with_changed_signed_facts_is_invalid_locally() {
        let first_lease = latest_lease(&issued_state());
        let first = verified(&first_lease);
        let scope = first.trusted_time_scope_id();
        let fresh = TrustedTimeStateV1::fresh(scope);
        let accepted =
            evaluate_offline_borrow(&fresh, T0 + 60, None, &first, time_policy());
        let OfflineBorrowDecision::WorkAllowed { next_state } = accepted else {
            panic!("first lease should work");
        };

        let mut changed = first_lease.clone();
        changed.lease_id = "conflicting-same-sequence".into();
        changed.new_paid_work_until += 60;
        changed.hard_valid_until += 60;
        let conflict = verified(&changed);

        assert_eq!(
            evaluate_offline_borrow(
                &next_state,
                T0 + 120,
                None,
                &conflict,
                time_policy(),
            ),
            OfflineBorrowDecision::Invalid(TrustedTimeError::LeaseSequenceConflict)
        );
    }
}
