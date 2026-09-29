use crate::{DEVICE_KEY_ID_LEN, MAX_ID_LEN, bounded_nonempty};
use thiserror::Error;

pub const MAX_ACTIVATION_HISTORY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationSlotState {
    Active,
    Released,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationSlotRecord {
    activation_id: String,
    device_key_id: [u8; DEVICE_KEY_ID_LEN],
    state: ActivationSlotState,
    predecessor_activation_id: Option<String>,
    recovery_id: Option<String>,
}

impl ActivationSlotRecord {
    pub fn activation_id(&self) -> &str {
        &self.activation_id
    }

    pub fn device_key_id(&self) -> &[u8; DEVICE_KEY_ID_LEN] {
        &self.device_key_id
    }

    pub fn state(&self) -> ActivationSlotState {
        self.state
    }

    pub fn predecessor_activation_id(&self) -> Option<&str> {
        self.predecessor_activation_id.as_deref()
    }

    pub fn recovery_id(&self) -> Option<&str> {
        self.recovery_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementActivationState {
    entitlement_id: String,
    generation: u64,
    slots: Vec<ActivationSlotRecord>,
}

impl EntitlementActivationState {
    pub fn with_active(
        entitlement_id: impl Into<String>,
        activation_id: impl Into<String>,
        device_key_id: [u8; DEVICE_KEY_ID_LEN],
    ) -> Result<Self, ActivationRecoveryError> {
        let entitlement_id = entitlement_id.into();
        let activation_id = activation_id.into();
        validate_id(&entitlement_id)?;
        validate_id(&activation_id)?;

        Ok(Self {
            entitlement_id,
            generation: 0,
            slots: vec![ActivationSlotRecord {
                activation_id,
                device_key_id,
                state: ActivationSlotState::Active,
                predecessor_activation_id: None,
                recovery_id: None,
            }],
        })
    }

    pub fn entitlement_id(&self) -> &str {
        &self.entitlement_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn slots(&self) -> &[ActivationSlotRecord] {
        &self.slots
    }

    pub fn active_slot(&self) -> Option<&ActivationSlotRecord> {
        self.slots
            .iter()
            .find(|slot| slot.state == ActivationSlotState::Active)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RebindRequestV1<'a> {
    pub recovery_id: &'a str,
    pub expected_old_activation_id: &'a str,
    pub new_activation_id: &'a str,
    pub new_device_key_id: [u8; DEVICE_KEY_ID_LEN],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationRecoveryOutcome {
    ReusedExisting { activation_id: String },
    Released { activation_id: String },
    AlreadyReleased { activation_id: String },
    Revoked { activation_id: String },
    AlreadyRevoked { activation_id: String },
    Rebound { activation_id: String },
    RebindAlreadyApplied { activation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationRecoveryTransition {
    outcome: ActivationRecoveryOutcome,
    next_state: EntitlementActivationState,
}

impl ActivationRecoveryTransition {
    pub fn outcome(&self) -> &ActivationRecoveryOutcome {
        &self.outcome
    }

    pub fn next_state(&self) -> &EntitlementActivationState {
        &self.next_state
    }

    pub fn into_next_state(self) -> EntitlementActivationState {
        self.next_state
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ActivationRecoveryError {
    #[error("activation recovery identifier violates V1 bounds")]
    InvalidIdentifier,
    #[error("activation history limit reached")]
    HistoryFull,
    #[error("activation state has no active slot")]
    NoActiveSlot,
    #[error("activation state contains multiple active slots")]
    InvalidState,
    #[error("surviving DeviceKey does not match the active activation")]
    DeviceMismatch,
    #[error("rebind must not replace an activation with the same DeviceKey")]
    DeviceAlreadyBound,
    #[error("expected activation is stale")]
    StaleActivation,
    #[error("new activation id already exists")]
    ActivationIdConflict,
    #[error("recovery id was already used with different facts")]
    RecoveryIdConflict,
}

fn validate_id(value: &str) -> Result<(), ActivationRecoveryError> {
    if bounded_nonempty(value, MAX_ID_LEN) {
        Ok(())
    } else {
        Err(ActivationRecoveryError::InvalidIdentifier)
    }
}

fn active_index(state: &EntitlementActivationState) -> Result<usize, ActivationRecoveryError> {
    let mut active = state
        .slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.state == ActivationSlotState::Active);
    let (index, _) = active
        .next()
        .ok_or(ActivationRecoveryError::NoActiveSlot)?;
    if active.next().is_some() {
        return Err(ActivationRecoveryError::InvalidState);
    }
    Ok(index)
}

pub fn reuse_surviving_device(
    state: &EntitlementActivationState,
    device_key_id: &[u8; DEVICE_KEY_ID_LEN],
) -> Result<ActivationRecoveryTransition, ActivationRecoveryError> {
    let index = active_index(state)?;
    let active = &state.slots[index];
    if &active.device_key_id != device_key_id {
        return Err(ActivationRecoveryError::DeviceMismatch);
    }

    Ok(ActivationRecoveryTransition {
        outcome: ActivationRecoveryOutcome::ReusedExisting {
            activation_id: active.activation_id.clone(),
        },
        next_state: state.clone(),
    })
}

pub fn release_activation(
    state: &EntitlementActivationState,
    expected_activation_id: &str,
) -> Result<ActivationRecoveryTransition, ActivationRecoveryError> {
    validate_id(expected_activation_id)?;

    if let Some(existing) = state
        .slots
        .iter()
        .find(|slot| slot.activation_id == expected_activation_id)
        && existing.state == ActivationSlotState::Released
    {
        return Ok(ActivationRecoveryTransition {
            outcome: ActivationRecoveryOutcome::AlreadyReleased {
                activation_id: existing.activation_id.clone(),
            },
            next_state: state.clone(),
        });
    }

    let index = active_index(state)?;
    if state.slots[index].activation_id != expected_activation_id {
        return Err(ActivationRecoveryError::StaleActivation);
    }

    let mut next = state.clone();
    next.slots[index].state = ActivationSlotState::Released;
    next.generation = next.generation.saturating_add(1);
    Ok(ActivationRecoveryTransition {
        outcome: ActivationRecoveryOutcome::Released {
            activation_id: expected_activation_id.to_owned(),
        },
        next_state: next,
    })
}

pub fn revoke_activation(
    state: &EntitlementActivationState,
    expected_activation_id: &str,
) -> Result<ActivationRecoveryTransition, ActivationRecoveryError> {
    validate_id(expected_activation_id)?;

    if let Some(existing) = state
        .slots
        .iter()
        .find(|slot| slot.activation_id == expected_activation_id)
        && existing.state == ActivationSlotState::Revoked
    {
        return Ok(ActivationRecoveryTransition {
            outcome: ActivationRecoveryOutcome::AlreadyRevoked {
                activation_id: existing.activation_id.clone(),
            },
            next_state: state.clone(),
        });
    }

    let index = active_index(state)?;
    if state.slots[index].activation_id != expected_activation_id {
        return Err(ActivationRecoveryError::StaleActivation);
    }

    let mut next = state.clone();
    next.slots[index].state = ActivationSlotState::Revoked;
    next.generation = next.generation.saturating_add(1);
    Ok(ActivationRecoveryTransition {
        outcome: ActivationRecoveryOutcome::Revoked {
            activation_id: expected_activation_id.to_owned(),
        },
        next_state: next,
    })
}

pub fn rebind_activation(
    state: &EntitlementActivationState,
    request: RebindRequestV1<'_>,
) -> Result<ActivationRecoveryTransition, ActivationRecoveryError> {
    validate_id(request.recovery_id)?;
    validate_id(request.expected_old_activation_id)?;
    validate_id(request.new_activation_id)?;

    if let Some(existing) = state
        .slots
        .iter()
        .find(|slot| slot.recovery_id.as_deref() == Some(request.recovery_id))
    {
        let exact_replay = existing.activation_id == request.new_activation_id
            && existing.predecessor_activation_id.as_deref()
                == Some(request.expected_old_activation_id)
            && existing.device_key_id == request.new_device_key_id;
        if exact_replay {
            return Ok(ActivationRecoveryTransition {
                outcome: ActivationRecoveryOutcome::RebindAlreadyApplied {
                    activation_id: existing.activation_id.clone(),
                },
                next_state: state.clone(),
            });
        }
        return Err(ActivationRecoveryError::RecoveryIdConflict);
    }

    if state
        .slots
        .iter()
        .any(|slot| slot.activation_id == request.new_activation_id)
    {
        return Err(ActivationRecoveryError::ActivationIdConflict);
    }
    if state.slots.len() >= MAX_ACTIVATION_HISTORY {
        return Err(ActivationRecoveryError::HistoryFull);
    }

    let active_index = active_index(state)?;
    let active = &state.slots[active_index];
    if active.activation_id != request.expected_old_activation_id {
        return Err(ActivationRecoveryError::StaleActivation);
    }
    if active.device_key_id == request.new_device_key_id {
        return Err(ActivationRecoveryError::DeviceAlreadyBound);
    }

    let mut next = state.clone();
    next.slots[active_index].state = ActivationSlotState::Released;
    next.slots.push(ActivationSlotRecord {
        activation_id: request.new_activation_id.to_owned(),
        device_key_id: request.new_device_key_id,
        state: ActivationSlotState::Active,
        predecessor_activation_id: Some(request.expected_old_activation_id.to_owned()),
        recovery_id: Some(request.recovery_id.to_owned()),
    });
    next.generation = next.generation.saturating_add(1);

    Ok(ActivationRecoveryTransition {
        outcome: ActivationRecoveryOutcome::Rebound {
            activation_id: request.new_activation_id.to_owned(),
        },
        next_state: next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE_A: [u8; DEVICE_KEY_ID_LEN] = [0x11; DEVICE_KEY_ID_LEN];
    const DEVICE_B: [u8; DEVICE_KEY_ID_LEN] = [0x22; DEVICE_KEY_ID_LEN];
    const DEVICE_C: [u8; DEVICE_KEY_ID_LEN] = [0x33; DEVICE_KEY_ID_LEN];

    fn initial() -> EntitlementActivationState {
        EntitlementActivationState::with_active("ent-1", "act-a", DEVICE_A)
            .expect("initial activation state")
    }

    fn rebind<'a>(
        state: &'a EntitlementActivationState,
        recovery_id: &'a str,
        expected: &'a str,
        new_activation: &'a str,
        new_device: [u8; DEVICE_KEY_ID_LEN],
    ) -> Result<ActivationRecoveryTransition, ActivationRecoveryError> {
        rebind_activation(
            state,
            RebindRequestV1 {
                recovery_id,
                expected_old_activation_id: expected,
                new_activation_id: new_activation,
                new_device_key_id: new_device,
            },
        )
    }

    #[test]
    fn surviving_device_reuses_same_activation_without_consuming_slot() {
        let state = initial();
        let transition =
            reuse_surviving_device(&state, &DEVICE_A).expect("surviving key must reuse");

        assert!(matches!(
            transition.outcome(),
            ActivationRecoveryOutcome::ReusedExisting { activation_id }
                if activation_id == "act-a"
        ));
        assert_eq!(transition.next_state(), &state);
        assert_eq!(transition.next_state().generation(), 0);
        assert_eq!(transition.next_state().slots().len(), 1);
    }

    #[test]
    fn release_is_idempotent_and_retains_history() {
        let state = initial();
        let released = release_activation(&state, "act-a")
            .expect("release active slot")
            .into_next_state();
        assert_eq!(released.generation(), 1);
        assert_eq!(released.slots().len(), 1);
        assert_eq!(released.slots()[0].state(), ActivationSlotState::Released);

        let replay = release_activation(&released, "act-a").expect("idempotent release");
        assert!(matches!(
            replay.outcome(),
            ActivationRecoveryOutcome::AlreadyReleased { .. }
        ));
        assert_eq!(replay.next_state(), &released);
    }

    #[test]
    fn rebind_releases_old_and_creates_new_activation_lineage() {
        let state = initial();
        let rebound = rebind(&state, "recover-1", "act-a", "act-b", DEVICE_B)
            .expect("rebind")
            .into_next_state();

        assert_eq!(rebound.entitlement_id(), "ent-1");
        assert_eq!(rebound.generation(), 1);
        assert_eq!(rebound.slots().len(), 2);
        assert_eq!(rebound.slots()[0].state(), ActivationSlotState::Released);
        let active = rebound.active_slot().expect("new active slot");
        assert_eq!(active.activation_id(), "act-b");
        assert_eq!(active.device_key_id(), &DEVICE_B);
        assert_eq!(active.predecessor_activation_id(), Some("act-a"));
        assert_eq!(active.recovery_id(), Some("recover-1"));
    }

    #[test]
    fn exact_rebind_retry_is_idempotent_and_does_not_consume_second_seat() {
        let state = initial();
        let rebound = rebind(&state, "recover-1", "act-a", "act-b", DEVICE_B)
            .expect("first rebind")
            .into_next_state();

        let replay = rebind(&rebound, "recover-1", "act-a", "act-b", DEVICE_B)
            .expect("exact retry");
        assert!(matches!(
            replay.outcome(),
            ActivationRecoveryOutcome::RebindAlreadyApplied { activation_id }
                if activation_id == "act-b"
        ));
        assert_eq!(replay.next_state(), &rebound);
        assert_eq!(replay.next_state().slots().len(), 2);
        assert_eq!(replay.next_state().generation(), 1);
    }

    #[test]
    fn recovery_id_reuse_with_different_facts_fails_closed() {
        let state = initial();
        let rebound = rebind(&state, "recover-1", "act-a", "act-b", DEVICE_B)
            .expect("first rebind")
            .into_next_state();

        let error = rebind(&rebound, "recover-1", "act-a", "act-c", DEVICE_C)
            .expect_err("conflicting retry");
        assert_eq!(error, ActivationRecoveryError::RecoveryIdConflict);
    }

    #[test]
    fn stale_recovery_cannot_replace_newer_active_slot() {
        let state = initial();
        let rebound = rebind(&state, "recover-1", "act-a", "act-b", DEVICE_B)
            .expect("first rebind")
            .into_next_state();

        let before = rebound.clone();
        let error = rebind(&rebound, "recover-2", "act-a", "act-c", DEVICE_C)
            .expect_err("stale old activation");
        assert_eq!(error, ActivationRecoveryError::StaleActivation);
        assert_eq!(rebound, before);
    }

    #[test]
    fn same_device_rebind_is_rejected_in_favor_of_reuse_path() {
        let state = initial();
        let error = rebind(&state, "recover-1", "act-a", "act-b", DEVICE_A)
            .expect_err("same DeviceKey must not allocate another activation");
        assert_eq!(error, ActivationRecoveryError::DeviceAlreadyBound);

        let reuse = reuse_surviving_device(&state, &DEVICE_A).expect("reuse");
        assert_eq!(reuse.next_state().slots().len(), 1);
    }

    #[test]
    fn revoke_is_terminal_and_idempotent_while_retaining_history() {
        let state = initial();
        let revoked = revoke_activation(&state, "act-a")
            .expect("revoke")
            .into_next_state();
        assert_eq!(revoked.slots()[0].state(), ActivationSlotState::Revoked);
        assert_eq!(revoked.generation(), 1);

        let replay = revoke_activation(&revoked, "act-a").expect("idempotent revoke");
        assert!(matches!(
            replay.outcome(),
            ActivationRecoveryOutcome::AlreadyRevoked { .. }
        ));
        assert_eq!(replay.next_state(), &revoked);
    }
}
