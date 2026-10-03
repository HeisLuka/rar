use crate::{DEVICE_KEY_ID_LEN, MAX_ID_LEN, bounded_nonempty};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const LEASE_COMMITMENT_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedTimeStateV1 {
    pub generation: u64,
    pub device_key_id: [u8; DEVICE_KEY_ID_LEN],
    pub server_time_high_water: Option<i64>,
    pub local_wall_high_water: Option<i64>,
    pub highest_lease_sequence: Option<u64>,
    pub last_lease_id: Option<String>,
    pub last_lease_issued_at: Option<i64>,
    pub last_lease_not_before: Option<i64>,
    pub last_lease_valid_until: Option<i64>,
    pub last_lease_offline_grace_until: Option<i64>,
    pub last_lease_commitment: Option<[u8; LEASE_COMMITMENT_LEN]>,
}

impl TrustedTimeStateV1 {
    pub fn fresh(device_key_id: [u8; DEVICE_KEY_ID_LEN]) -> Self {
        Self {
            generation: 0,
            device_key_id,
            server_time_high_water: None,
            local_wall_high_water: None,
            highest_lease_sequence: None,
            last_lease_id: None,
            last_lease_issued_at: None,
            last_lease_not_before: None,
            last_lease_valid_until: None,
            last_lease_offline_grace_until: None,
            last_lease_commitment: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimePolicy {
    pub skew_seconds: i64,
    pub forward_jump_limit_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseTimeInputV1<'a> {
    pub lease_id: &'a str,
    pub lease_sequence: u64,
    pub issued_at: i64,
    pub not_before: i64,
    pub valid_until: i64,
    pub offline_grace_until: Option<i64>,
    pub authenticated_server_time: Option<i64>,
    pub lease_commitment: [u8; LEASE_COMMITMENT_LEN],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeAcceptance {
    Valid {
        next_state: TrustedTimeStateV1,
    },
    Grace {
        until: i64,
        next_state: TrustedTimeStateV1,
    },
}

impl TimeAcceptance {
    pub fn next_state(&self) -> &TrustedTimeStateV1 {
        match self {
            Self::Valid { next_state } | Self::Grace { next_state, .. } => next_state,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrustedTimeError {
    #[error("invalid trusted-time policy")]
    InvalidPolicy,
    #[error("invalid signed lease time shape")]
    InvalidLease,
    #[error("trusted-time state belongs to another device")]
    StateDeviceMismatch,
    #[error("older signed lease replayed")]
    LeaseReplay,
    #[error("same lease sequence carries different signed facts")]
    LeaseSequenceConflict,
    #[error("local clock moved backward beyond allowed skew")]
    ClockRollback,
    #[error("local clock jumped forward beyond configured confidence window")]
    TimeUncertain,
    #[error("signed lease is not yet valid")]
    NotYetValid,
    #[error("signed lease expired")]
    Expired,
}

fn validate_policy(policy: TimePolicy) -> Result<(), TrustedTimeError> {
    if policy.skew_seconds < 0
        || policy
            .forward_jump_limit_seconds
            .is_some_and(|limit| limit < 0)
    {
        return Err(TrustedTimeError::InvalidPolicy);
    }
    Ok(())
}

fn validate_lease(lease: &LeaseTimeInputV1<'_>) -> Result<(), TrustedTimeError> {
    if !bounded_nonempty(lease.lease_id, MAX_ID_LEN)
        || lease.lease_sequence == 0
        || lease.not_before > lease.valid_until
        || lease.issued_at > lease.valid_until
        || lease
            .offline_grace_until
            .is_some_and(|grace| grace < lease.valid_until)
    {
        return Err(TrustedTimeError::InvalidLease);
    }
    Ok(())
}

fn same_seen_lease(state: &TrustedTimeStateV1, lease: &LeaseTimeInputV1<'_>) -> bool {
    state.last_lease_id.as_deref() == Some(lease.lease_id)
        && state.last_lease_issued_at == Some(lease.issued_at)
        && state.last_lease_not_before == Some(lease.not_before)
        && state.last_lease_valid_until == Some(lease.valid_until)
        && state.last_lease_offline_grace_until == lease.offline_grace_until
        && state.last_lease_commitment == Some(lease.lease_commitment)
}

fn strongest_time_anchor(state: &TrustedTimeStateV1, lease: &LeaseTimeInputV1<'_>) -> i64 {
    let mut anchor = lease.issued_at;
    if let Some(value) = state.server_time_high_water {
        anchor = anchor.max(value);
    }
    if let Some(value) = state.local_wall_high_water {
        anchor = anchor.max(value);
    }
    if let Some(value) = lease.authenticated_server_time {
        anchor = anchor.max(value);
    }
    anchor
}

fn accepted_next_state(
    state: &TrustedTimeStateV1,
    now: i64,
    lease: &LeaseTimeInputV1<'_>,
) -> TrustedTimeStateV1 {
    let authenticated_time = lease
        .authenticated_server_time
        .unwrap_or(lease.issued_at)
        .max(lease.issued_at);

    let mut next = state.clone();
    next.generation = next.generation.saturating_add(1);
    next.server_time_high_water = Some(
        next.server_time_high_water
            .map_or(authenticated_time, |seen| seen.max(authenticated_time)),
    );
    next.local_wall_high_water = Some(
        next.local_wall_high_water
            .map_or(now, |seen| seen.max(now)),
    );
    next.highest_lease_sequence = Some(
        next.highest_lease_sequence
            .map_or(lease.lease_sequence, |seen| seen.max(lease.lease_sequence)),
    );
    next.last_lease_id = Some(lease.lease_id.to_owned());
    next.last_lease_issued_at = Some(lease.issued_at);
    next.last_lease_not_before = Some(lease.not_before);
    next.last_lease_valid_until = Some(lease.valid_until);
    next.last_lease_offline_grace_until = lease.offline_grace_until;
    next.last_lease_commitment = Some(lease.lease_commitment);
    next
}

/// Evaluates an already signature-verified subscription/trial lease against local
/// anti-rollback state. The input state is immutable; a caller only receives a
/// proposed next state after a VALID or explicit GRACE decision.
pub fn evaluate_time_bound_right(
    state: &TrustedTimeStateV1,
    expected_device_key_id: &[u8; DEVICE_KEY_ID_LEN],
    now: i64,
    lease: &LeaseTimeInputV1<'_>,
    policy: TimePolicy,
) -> Result<TimeAcceptance, TrustedTimeError> {
    validate_policy(policy)?;
    validate_lease(lease)?;

    if &state.device_key_id != expected_device_key_id {
        return Err(TrustedTimeError::StateDeviceMismatch);
    }

    if let Some(seen) = state.highest_lease_sequence {
        if lease.lease_sequence < seen {
            return Err(TrustedTimeError::LeaseReplay);
        }
        if lease.lease_sequence == seen && !same_seen_lease(state, lease) {
            return Err(TrustedTimeError::LeaseSequenceConflict);
        }
    }

    if let Some(high_water) = state.local_wall_high_water
        && now.saturating_add(policy.skew_seconds) < high_water
    {
        return Err(TrustedTimeError::ClockRollback);
    }

    if let Some(limit) = policy.forward_jump_limit_seconds {
        let anchor = strongest_time_anchor(state, lease);
        if now > anchor.saturating_add(limit) {
            return Err(TrustedTimeError::TimeUncertain);
        }
    }

    if now.saturating_add(policy.skew_seconds) < lease.not_before {
        return Err(TrustedTimeError::NotYetValid);
    }

    let next_state = accepted_next_state(state, now, lease);
    if now <= lease.valid_until.saturating_add(policy.skew_seconds) {
        return Ok(TimeAcceptance::Valid { next_state });
    }

    if let Some(until) = lease.offline_grace_until
        && now <= until.saturating_add(policy.skew_seconds)
    {
        return Ok(TimeAcceptance::Grace { until, next_state });
    }

    Err(TrustedTimeError::Expired)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: [u8; DEVICE_KEY_ID_LEN] = [0xA5; DEVICE_KEY_ID_LEN];

    fn policy() -> TimePolicy {
        TimePolicy {
            skew_seconds: 300,
            forward_jump_limit_seconds: Some(90 * 24 * 60 * 60),
        }
    }

    fn lease() -> LeaseTimeInputV1<'static> {
        LeaseTimeInputV1 {
            lease_id: "lease-100",
            lease_sequence: 100,
            issued_at: 1_800_000_000,
            not_before: 1_800_000_000,
            valid_until: 1_802_592_000,
            offline_grace_until: Some(1_802_678_400),
            authenticated_server_time: None,
            lease_commitment: [0x11; LEASE_COMMITMENT_LEN],
        }
    }

    fn accepted_state(now: i64) -> TrustedTimeStateV1 {
        let state = TrustedTimeStateV1::fresh(DEVICE);
        evaluate_time_bound_right(&state, &DEVICE, now, &lease(), policy())
            .unwrap()
            .next_state()
            .clone()
    }

    #[test]
    fn normal_progression_advances_both_high_water_marks() {
        let now = 1_800_003_600;
        let accepted = evaluate_time_bound_right(
            &TrustedTimeStateV1::fresh(DEVICE),
            &DEVICE,
            now,
            &lease(),
            policy(),
        )
        .unwrap();

        let next = accepted.next_state();
        assert_eq!(next.local_wall_high_water, Some(now));
        assert_eq!(next.server_time_high_water, Some(1_800_000_000));
        assert_eq!(next.highest_lease_sequence, Some(100));
        assert_eq!(next.generation, 1);
    }

    #[test]
    fn small_backward_adjustment_inside_skew_is_accepted_without_lowering_high_water() {
        let state = accepted_state(1_800_003_600);
        let now = 1_800_003_480;

        let accepted = evaluate_time_bound_right(&state, &DEVICE, now, &lease(), policy()).unwrap();
        assert_eq!(
            accepted.next_state().local_wall_high_water,
            Some(1_800_003_600)
        );
    }

    #[test]
    fn meaningful_clock_rollback_is_rejected_without_state_change() {
        let state = accepted_state(1_800_100_000);
        let before = state.clone();

        let err = evaluate_time_bound_right(&state, &DEVICE, 1_800_010_000, &lease(), policy())
            .unwrap_err();

        assert_eq!(err, TrustedTimeError::ClockRollback);
        assert_eq!(state, before);
    }

    #[test]
    fn extreme_forward_jump_is_uncertain_and_does_not_poison_high_water() {
        let state = accepted_state(1_800_003_600);
        let before = state.clone();

        let err = evaluate_time_bound_right(
            &state,
            &DEVICE,
            2_200_000_000,
            &lease(),
            policy(),
        )
        .unwrap_err();

        assert_eq!(err, TrustedTimeError::TimeUncertain);
        assert_eq!(state, before);
    }

    #[test]
    fn older_lease_sequence_is_rejected() {
        let state = accepted_state(1_800_003_600);
        let mut replay = lease();
        replay.lease_sequence = 99;
        replay.lease_id = "lease-99";
        replay.lease_commitment = [0x09; LEASE_COMMITMENT_LEN];

        let err = evaluate_time_bound_right(
            &state,
            &DEVICE,
            1_800_007_200,
            &replay,
            policy(),
        )
        .unwrap_err();

        assert_eq!(err, TrustedTimeError::LeaseReplay);
    }

    #[test]
    fn same_sequence_and_same_signed_commitment_is_idempotently_accepted() {
        let state = accepted_state(1_800_003_600);

        let accepted = evaluate_time_bound_right(
            &state,
            &DEVICE,
            1_800_007_200,
            &lease(),
            policy(),
        )
        .unwrap();

        assert_eq!(accepted.next_state().highest_lease_sequence, Some(100));
        assert_eq!(accepted.next_state().generation, 2);
    }

    #[test]
    fn same_sequence_with_different_signed_facts_fails_closed() {
        let state = accepted_state(1_800_003_600);
        let mut conflict = lease();
        conflict.lease_commitment = [0x22; LEASE_COMMITMENT_LEN];

        let err = evaluate_time_bound_right(
            &state,
            &DEVICE,
            1_800_007_200,
            &conflict,
            policy(),
        )
        .unwrap_err();

        assert_eq!(err, TrustedTimeError::LeaseSequenceConflict);
    }

    #[test]
    fn grace_is_explicit_and_advances_state_only_when_accepted() {
        let mut p = policy();
        p.forward_jump_limit_seconds = None;
        let now = lease().valid_until + p.skew_seconds + 1;

        let accepted = evaluate_time_bound_right(
            &TrustedTimeStateV1::fresh(DEVICE),
            &DEVICE,
            now,
            &lease(),
            p,
        )
        .unwrap();

        assert!(matches!(
            accepted,
            TimeAcceptance::Grace {
                until: 1_802_678_400,
                ..
            }
        ));
        assert_eq!(accepted.next_state().local_wall_high_water, Some(now));
    }

    #[test]
    fn expired_lease_does_not_advance_state() {
        let state = TrustedTimeStateV1::fresh(DEVICE);
        let before = state.clone();
        let mut p = policy();
        p.forward_jump_limit_seconds = None;
        let now = lease().offline_grace_until.unwrap() + p.skew_seconds + 1;

        let err = evaluate_time_bound_right(&state, &DEVICE, now, &lease(), p).unwrap_err();

        assert_eq!(err, TrustedTimeError::Expired);
        assert_eq!(state, before);
    }

    #[test]
    fn not_yet_valid_lease_is_rejected() {
        let state = TrustedTimeStateV1::fresh(DEVICE);
        let now = lease().not_before - policy().skew_seconds - 1;

        let err = evaluate_time_bound_right(&state, &DEVICE, now, &lease(), policy()).unwrap_err();

        assert_eq!(err, TrustedTimeError::NotYetValid);
    }

    #[test]
    fn malformed_window_and_device_scope_mismatch_fail_closed() {
        let state = TrustedTimeStateV1::fresh(DEVICE);
        let mut malformed = lease();
        malformed.offline_grace_until = Some(malformed.valid_until - 1);
        let err =
            evaluate_time_bound_right(&state, &DEVICE, malformed.issued_at, &malformed, policy())
                .unwrap_err();
        assert_eq!(err, TrustedTimeError::InvalidLease);

        let other_device = [0x5A; DEVICE_KEY_ID_LEN];
        let err =
            evaluate_time_bound_right(&state, &other_device, lease().issued_at, &lease(), policy())
                .unwrap_err();
        assert_eq!(err, TrustedTimeError::StateDeviceMismatch);
    }
}
