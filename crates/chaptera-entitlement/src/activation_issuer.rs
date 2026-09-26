use crate::{
    DEVICE_KEY_ID_LEN, MAX_GRANT_LEN, MAX_GRANTS, MAX_ID_LEN, MAX_PRODUCT_ID_LEN,
    VerifiedActivationRequest, bounded_nonempty,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_ISSUER_ACTIVE_SLOTS: usize = 64;
pub const MAX_ISSUER_REQUEST_RECEIPTS: usize = 512;
const REQUEST_COMMITMENT_DOMAIN: &[u8] =
    b"Chaptera.ActivationIssuer.VerifiedRequest.v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssuerSlot {
    slot_id: String,
    activation_id: String,
    device_key_id: [u8; DEVICE_KEY_ID_LEN],
}

impl ActivationIssuerSlot {
    pub fn slot_id(&self) -> &str {
        &self.slot_id
    }

    pub fn activation_id(&self) -> &str {
        &self.activation_id
    }

    pub fn device_key_id(&self) -> &[u8; DEVICE_KEY_ID_LEN] {
        &self.device_key_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssuance {
    request_id: String,
    slot_id: String,
    activation_id: String,
    entitlement_id: String,
    product_id: String,
    requested_major: u32,
    device_key_id: [u8; DEVICE_KEY_ID_LEN],
    grants: Vec<String>,
}

impl ActivationIssuance {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn slot_id(&self) -> &str {
        &self.slot_id
    }

    pub fn activation_id(&self) -> &str {
        &self.activation_id
    }

    pub fn entitlement_id(&self) -> &str {
        &self.entitlement_id
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn requested_major(&self) -> u32 {
        self.requested_major
    }

    pub fn device_key_id(&self) -> &[u8; DEVICE_KEY_ID_LEN] {
        &self.device_key_id
    }

    pub fn grants(&self) -> &[String] {
        &self.grants
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationIssuanceDisposition {
    AllocatedNewSlot,
    ReusedExistingSlot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssuanceOutcome {
    disposition: ActivationIssuanceDisposition,
    issuance: ActivationIssuance,
}

impl ActivationIssuanceOutcome {
    pub fn disposition(&self) -> ActivationIssuanceDisposition {
        self.disposition
    }

    pub fn issuance(&self) -> &ActivationIssuance {
        &self.issuance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestReceipt {
    request_id: String,
    request_commitment: [u8; 32],
    outcome: ActivationIssuanceOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssuerState {
    entitlement_id: String,
    product_id: String,
    grants: Vec<String>,
    max_active_slots: usize,
    generation: u64,
    active_slots: Vec<ActivationIssuerSlot>,
    receipts: Vec<RequestReceipt>,
}

impl ActivationIssuerState {
    pub fn new(
        entitlement_id: impl Into<String>,
        product_id: impl Into<String>,
        grants: Vec<String>,
        max_active_slots: usize,
    ) -> Result<Self, ActivationIssuerError> {
        let entitlement_id = entitlement_id.into();
        let product_id = product_id.into();
        validate_id(&entitlement_id)?;
        if !bounded_nonempty(&product_id, MAX_PRODUCT_ID_LEN) {
            return Err(ActivationIssuerError::InvalidPolicy);
        }
        validate_grants(&grants)?;
        if max_active_slots == 0 || max_active_slots > MAX_ISSUER_ACTIVE_SLOTS {
            return Err(ActivationIssuerError::InvalidPolicy);
        }

        Ok(Self {
            entitlement_id,
            product_id,
            grants,
            max_active_slots,
            generation: 0,
            active_slots: Vec::new(),
            receipts: Vec::new(),
        })
    }

    pub fn entitlement_id(&self) -> &str {
        &self.entitlement_id
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn max_active_slots(&self) -> usize {
        self.max_active_slots
    }

    pub fn active_slots(&self) -> &[ActivationIssuerSlot] {
        &self.active_slots
    }

    pub fn request_receipt_count(&self) -> usize {
        self.receipts.len()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProposedActivationIds<'a> {
    pub slot_id: &'a str,
    pub activation_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssuerTransition {
    outcome: ActivationIssuanceOutcome,
    next_state: ActivationIssuerState,
}

impl ActivationIssuerTransition {
    pub fn outcome(&self) -> &ActivationIssuanceOutcome {
        &self.outcome
    }

    pub fn next_state(&self) -> &ActivationIssuerState {
        &self.next_state
    }

    pub fn into_next_state(self) -> ActivationIssuerState {
        self.next_state
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ActivationIssuerError {
    #[error("activation issuer identifier violates V1 bounds")]
    InvalidIdentifier,
    #[error("activation issuer policy is invalid")]
    InvalidPolicy,
    #[error("verified request product does not match issuer authority")]
    ProductMismatch,
    #[error("request_id was reused with different verified request facts")]
    RequestIdConflict,
    #[error("issuer authority generation is stale")]
    StaleAuthorityGeneration,
    #[error("no ActivationSlot capacity remains")]
    NoCapacity,
    #[error("proposed slot id already exists")]
    SlotIdConflict,
    #[error("proposed activation id already exists")]
    ActivationIdConflict,
    #[error("request receipt history is full")]
    ReceiptHistoryFull,
    #[error("issuer authority generation is exhausted")]
    GenerationExhausted,
}

fn validate_id(value: &str) -> Result<(), ActivationIssuerError> {
    if bounded_nonempty(value, MAX_ID_LEN) {
        Ok(())
    } else {
        Err(ActivationIssuerError::InvalidIdentifier)
    }
}

fn validate_grants(grants: &[String]) -> Result<(), ActivationIssuerError> {
    if grants.is_empty()
        || grants.len() > MAX_GRANTS
        || grants
            .iter()
            .any(|grant| !bounded_nonempty(grant, MAX_GRANT_LEN))
    {
        return Err(ActivationIssuerError::InvalidPolicy);
    }
    for (index, grant) in grants.iter().enumerate() {
        if grants[..index].iter().any(|seen| seen == grant) {
            return Err(ActivationIssuerError::InvalidPolicy);
        }
    }
    Ok(())
}

fn hash_len_prefixed(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn verified_request_commitment(request: &VerifiedActivationRequest) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(REQUEST_COMMITMENT_DOMAIN);
    hash_len_prefixed(&mut hash, request.request_id().as_bytes());
    hash_len_prefixed(&mut hash, request.product_id().as_bytes());
    hash.update(request.requested_major().to_be_bytes());
    hash.update(request.request_nonce());
    hash.update(request.device_key_id());
    hash.update(request.device_public_key_sec1());
    hash.finalize().into()
}

fn make_issuance(
    state: &ActivationIssuerState,
    request: &VerifiedActivationRequest,
    slot: &ActivationIssuerSlot,
) -> ActivationIssuance {
    ActivationIssuance {
        request_id: request.request_id().to_owned(),
        slot_id: slot.slot_id.clone(),
        activation_id: slot.activation_id.clone(),
        entitlement_id: state.entitlement_id.clone(),
        product_id: state.product_id.clone(),
        requested_major: request.requested_major(),
        device_key_id: *request.device_key_id(),
        grants: state.grants.clone(),
    }
}

fn next_generation(state: &ActivationIssuerState) -> Result<u64, ActivationIssuerError> {
    state
        .generation
        .checked_add(1)
        .ok_or(ActivationIssuerError::GenerationExhausted)
}

fn push_receipt(
    state: &mut ActivationIssuerState,
    request_id: String,
    request_commitment: [u8; 32],
    outcome: ActivationIssuanceOutcome,
) -> Result<(), ActivationIssuerError> {
    if state.receipts.len() >= MAX_ISSUER_REQUEST_RECEIPTS {
        return Err(ActivationIssuerError::ReceiptHistoryFull);
    }
    state.receipts.push(RequestReceipt {
        request_id,
        request_commitment,
        outcome,
    });
    Ok(())
}

/// Commits one already-verified ActivationRequest into entitlement/slot authority.
///
/// The request receipt lookup deliberately happens before generation and proposed-ID
/// checks. A retry after a lost response therefore returns the exact committed
/// issuance even when the caller has a stale generation or generated fresh candidate
/// IDs. request_nonce participates in the verified-facts commitment but is not the
/// idempotency or capacity authority: request_id is.
pub fn issue_verified_activation(
    state: &ActivationIssuerState,
    expected_generation: u64,
    request: &VerifiedActivationRequest,
    proposed: ProposedActivationIds<'_>,
) -> Result<ActivationIssuerTransition, ActivationIssuerError> {
    if request.product_id() != state.product_id {
        return Err(ActivationIssuerError::ProductMismatch);
    }

    let commitment = verified_request_commitment(request);
    if let Some(receipt) = state
        .receipts
        .iter()
        .find(|receipt| receipt.request_id == request.request_id())
    {
        if receipt.request_commitment != commitment {
            return Err(ActivationIssuerError::RequestIdConflict);
        }
        return Ok(ActivationIssuerTransition {
            outcome: receipt.outcome.clone(),
            next_state: state.clone(),
        });
    }

    if expected_generation != state.generation {
        return Err(ActivationIssuerError::StaleAuthorityGeneration);
    }

    if let Some(existing) = state
        .active_slots
        .iter()
        .find(|slot| slot.device_key_id == *request.device_key_id())
    {
        let outcome = ActivationIssuanceOutcome {
            disposition: ActivationIssuanceDisposition::ReusedExistingSlot,
            issuance: make_issuance(state, request, existing),
        };
        let mut next = state.clone();
        next.generation = next_generation(state)?;
        push_receipt(
            &mut next,
            request.request_id().to_owned(),
            commitment,
            outcome.clone(),
        )?;
        return Ok(ActivationIssuerTransition {
            outcome,
            next_state: next,
        });
    }

    validate_id(proposed.slot_id)?;
    validate_id(proposed.activation_id)?;

    if state.active_slots.len() >= state.max_active_slots {
        return Err(ActivationIssuerError::NoCapacity);
    }
    if state
        .active_slots
        .iter()
        .any(|slot| slot.slot_id == proposed.slot_id)
    {
        return Err(ActivationIssuerError::SlotIdConflict);
    }
    if state
        .active_slots
        .iter()
        .any(|slot| slot.activation_id == proposed.activation_id)
    {
        return Err(ActivationIssuerError::ActivationIdConflict);
    }

    let slot = ActivationIssuerSlot {
        slot_id: proposed.slot_id.to_owned(),
        activation_id: proposed.activation_id.to_owned(),
        device_key_id: *request.device_key_id(),
    };
    let outcome = ActivationIssuanceOutcome {
        disposition: ActivationIssuanceDisposition::AllocatedNewSlot,
        issuance: make_issuance(state, request, &slot),
    };

    let mut next = state.clone();
    next.generation = next_generation(state)?;
    next.active_slots.push(slot);
    push_receipt(
        &mut next,
        request.request_id().to_owned(),
        commitment,
        outcome.clone(),
    )?;

    Ok(ActivationIssuerTransition {
        outcome,
        next_state: next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_request::{
        ACTIVATION_REQUEST_SIGNATURE_DOMAIN, ActivationRequestFactsV1,
        assemble_activation_request, encode_activation_request_facts,
    };
    use crate::verify_activation_request;
    use p256::ecdsa::{Signature, SigningKey, signature::Signer};

    const PRODUCT: &str = "chaptera.editor";

    fn verified_request(
        request_id: &str,
        signing_byte: u8,
        nonce: [u8; 32],
        requested_major: u32,
    ) -> VerifiedActivationRequest {
        let signing =
            SigningKey::from_slice(&[signing_byte; 32]).expect("synthetic DeviceKey");
        let public = signing
            .verifying_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec();
        let device_key_id: [u8; DEVICE_KEY_ID_LEN] = Sha256::digest(&public).into();
        let facts = ActivationRequestFactsV1 {
            schema_version: 1,
            request_id: request_id.into(),
            product_id: PRODUCT.into(),
            requested_major,
            request_nonce: nonce,
            device_key_id,
            device_public_key_sec1: public,
        };
        let signed_facts = encode_activation_request_facts(&facts).expect("request facts");
        let mut message = Vec::new();
        message.extend_from_slice(ACTIVATION_REQUEST_SIGNATURE_DOMAIN);
        message.extend_from_slice(&signed_facts);
        let signature: Signature = signing.sign(&message);
        let signature_bytes: [u8; 64] = signature.to_bytes().into();
        let artifact =
            assemble_activation_request(signed_facts, signature_bytes).expect("request");
        verify_activation_request(&artifact, PRODUCT).expect("verified request")
    }

    fn authority(capacity: usize) -> ActivationIssuerState {
        ActivationIssuerState::new(
            "ent-1",
            PRODUCT,
            vec!["edit".into(), "export".into()],
            capacity,
        )
        .expect("issuer authority")
    }

    #[test]
    fn exact_request_replay_returns_same_slot_and_activation_despite_stale_retry_context() {
        let request = verified_request("req-1", 0x31, [0x91; 32], 2);
        let first = issue_verified_activation(
            &authority(1),
            0,
            &request,
            ProposedActivationIds {
                slot_id: "slot-1",
                activation_id: "act-1",
            },
        )
        .expect("first issuance");
        let committed = first.next_state().clone();

        let replay = issue_verified_activation(
            &committed,
            0,
            &request,
            ProposedActivationIds {
                slot_id: "slot-should-be-ignored",
                activation_id: "act-should-be-ignored",
            },
        )
        .expect("exact replay");

        assert_eq!(replay.outcome(), first.outcome());
        assert_eq!(replay.next_state(), &committed);
        assert_eq!(replay.outcome().issuance().slot_id(), "slot-1");
        assert_eq!(replay.outcome().issuance().activation_id(), "act-1");
        assert_eq!(committed.active_slots().len(), 1);
        assert_eq!(committed.request_receipt_count(), 1);
    }

    #[test]
    fn same_request_id_with_changed_verified_facts_fails_closed() {
        let first_request = verified_request("req-1", 0x31, [0x91; 32], 2);
        let state = issue_verified_activation(
            &authority(2),
            0,
            &first_request,
            ProposedActivationIds {
                slot_id: "slot-1",
                activation_id: "act-1",
            },
        )
        .unwrap()
        .into_next_state();

        let changed_nonce = verified_request("req-1", 0x31, [0x92; 32], 2);
        assert_eq!(
            issue_verified_activation(
                &state,
                state.generation(),
                &changed_nonce,
                ProposedActivationIds {
                    slot_id: "slot-2",
                    activation_id: "act-2",
                },
            )
            .unwrap_err(),
            ActivationIssuerError::RequestIdConflict
        );

        let changed_major = verified_request("req-1", 0x31, [0x91; 32], 3);
        assert_eq!(
            issue_verified_activation(
                &state,
                state.generation(),
                &changed_major,
                ProposedActivationIds {
                    slot_id: "slot-3",
                    activation_id: "act-3",
                },
            )
            .unwrap_err(),
            ActivationIssuerError::RequestIdConflict
        );
    }

    #[test]
    fn concurrent_capacity_one_requests_have_one_winner_and_never_overallocate() {
        let initial = authority(1);
        let request_a = verified_request("req-a", 0x31, [0x11; 32], 2);
        let request_b = verified_request("req-b", 0x32, [0x22; 32], 2);

        let winner = issue_verified_activation(
            &initial,
            0,
            &request_a,
            ProposedActivationIds {
                slot_id: "slot-a",
                activation_id: "act-a",
            },
        )
        .unwrap()
        .into_next_state();

        assert_eq!(
            issue_verified_activation(
                &winner,
                0,
                &request_b,
                ProposedActivationIds {
                    slot_id: "slot-b",
                    activation_id: "act-b",
                },
            )
            .unwrap_err(),
            ActivationIssuerError::StaleAuthorityGeneration
        );

        assert_eq!(
            issue_verified_activation(
                &winner,
                winner.generation(),
                &request_b,
                ProposedActivationIds {
                    slot_id: "slot-b",
                    activation_id: "act-b",
                },
            )
            .unwrap_err(),
            ActivationIssuerError::NoCapacity
        );
        assert_eq!(winner.active_slots().len(), 1);
    }

    #[test]
    fn new_request_from_already_bound_device_reuses_slot_instead_of_consuming_capacity() {
        let initial_request = verified_request("req-1", 0x31, [0x11; 32], 2);
        let state = issue_verified_activation(
            &authority(1),
            0,
            &initial_request,
            ProposedActivationIds {
                slot_id: "slot-1",
                activation_id: "act-1",
            },
        )
        .unwrap()
        .into_next_state();

        let same_device_new_request = verified_request("req-2", 0x31, [0x22; 32], 2);
        let transition = issue_verified_activation(
            &state,
            state.generation(),
            &same_device_new_request,
            ProposedActivationIds {
                slot_id: "slot-unused",
                activation_id: "act-unused",
            },
        )
        .expect("same DeviceKey reuses active binding");

        assert_eq!(
            transition.outcome().disposition(),
            ActivationIssuanceDisposition::ReusedExistingSlot
        );
        assert_eq!(transition.outcome().issuance().slot_id(), "slot-1");
        assert_eq!(transition.outcome().issuance().activation_id(), "act-1");
        assert_eq!(transition.next_state().active_slots().len(), 1);
        assert_eq!(transition.next_state().request_receipt_count(), 2);
    }

    #[test]
    fn nonce_is_signed_freshness_material_not_slot_idempotency_authority() {
        let same_nonce = [0x55; 32];
        let request_a = verified_request("req-a", 0x31, same_nonce, 2);
        let request_b = verified_request("req-b", 0x32, same_nonce, 2);
        let first = issue_verified_activation(
            &authority(2),
            0,
            &request_a,
            ProposedActivationIds {
                slot_id: "slot-a",
                activation_id: "act-a",
            },
        )
        .unwrap()
        .into_next_state();
        let second = issue_verified_activation(
            &first,
            first.generation(),
            &request_b,
            ProposedActivationIds {
                slot_id: "slot-b",
                activation_id: "act-b",
            },
        )
        .unwrap()
        .into_next_state();

        assert_eq!(second.active_slots().len(), 2);
        assert_eq!(second.request_receipt_count(), 2);
        assert_ne!(
            second.active_slots()[0].device_key_id(),
            second.active_slots()[1].device_key_id()
        );
    }
}
