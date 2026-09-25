#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenStatus {
    Supported,
    Partial,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueEvent {
    DocumentOpened {
        status: OpenStatus,
        page_count: usize,
        text_searchable: bool,
        initial_page: usize,
    },
    PageNavigated {
        page_index: usize,
    },
    SearchResultSelected {
        match_count: usize,
    },
    SearchMatchCopied,
    FullStoryCopied,
    WorkflowFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueReceiptKind {
    Reading { text_searchable: bool },
    SearchMatches { match_count: usize },
    TextCopied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValueReceipt {
    pub(crate) page_count: usize,
    pub(crate) kind: ValueReceiptKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupporterAction {
    Support,
    Later,
    AlreadySupported,
    Share,
    Report,
    ArchiveHelp,
}

#[derive(Debug, Clone, Copy)]
struct DocumentSession {
    page_count: usize,
    text_searchable: bool,
    initial_page: usize,
    visited_other_page: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ValueTracker {
    document: Option<DocumentSession>,
    receipt: Option<ValueReceipt>,
}

impl ValueTracker {
    pub(crate) fn observe(&mut self, event: ValueEvent) {
        match event {
            ValueEvent::DocumentOpened {
                status,
                page_count,
                text_searchable,
                initial_page,
            } => {
                self.receipt = None;
                self.document = matches!(status, OpenStatus::Supported | OpenStatus::Partial)
                    .then_some(DocumentSession {
                        page_count,
                        text_searchable,
                        initial_page,
                        visited_other_page: false,
                    })
                    .filter(|session| session.page_count > 0);
            }
            ValueEvent::PageNavigated { page_index } => {
                let Some(document) = self.document.as_mut() else {
                    return;
                };

                if page_index != document.initial_page && !document.visited_other_page {
                    document.visited_other_page = true;
                    return;
                }

                if document.visited_other_page {
                    let text_searchable = document.text_searchable;
                    self.grant(ValueReceiptKind::Reading { text_searchable });
                }
            }
            ValueEvent::SearchResultSelected { match_count } => {
                if match_count > 0 {
                    self.grant(ValueReceiptKind::SearchMatches { match_count });
                }
            }
            ValueEvent::SearchMatchCopied | ValueEvent::FullStoryCopied => {
                self.grant(ValueReceiptKind::TextCopied);
            }
            ValueEvent::WorkflowFailed => {
                self.document = None;
                self.receipt = None;
            }
        }
    }

    pub(crate) fn is_eligible(&self) -> bool {
        self.receipt.is_some()
    }

    pub(crate) fn receipt(&self) -> Option<ValueReceipt> {
        self.receipt
    }

    fn grant(&mut self, kind: ValueReceiptKind) {
        if self.receipt.is_some() {
            return;
        }

        if let Some(document) = self.document {
            self.receipt = Some(ValueReceipt {
                page_count: document.page_count,
                kind,
            });
        }
    }
}

const DAY_SECONDS: i64 = 24 * 60 * 60;
const PROMPT_WINDOW_SECONDS: i64 = 30 * DAY_SECONDS;
const MIN_PROMPT_COOLDOWN_SECONDS: i64 = 7 * DAY_SECONDS;
const REPEAT_DISMISS_SUPPRESSION_SECONDS: i64 = 45 * DAY_SECONDS;
const CLAIMED_SUPPORTED_SUPPRESSION_SECONDS: i64 = 180 * DAY_SECONDS;
const SUPPORT_CLICK_SUPPRESSION_SECONDS: i64 = 30 * DAY_SECONDS;
const MAX_PROMPTS_PER_WINDOW: usize = 2;
const REQUIRED_SUCCESSES_AFTER_PROMPT: u16 = 3;
const SUPPORTER_STATE_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupporterState {
    schema_version: u8,
    first_value_at_unix: Option<i64>,
    last_value_at_unix: Option<i64>,
    last_prompt_at_unix: Option<i64>,
    recent_prompt_unix: Vec<i64>,
    meaningful_successes_since_prompt: u16,
    dismiss_count: u16,
    dismiss_until_unix: Option<i64>,
    claimed_supported_until_unix: Option<i64>,
    support_clicked_until_unix: Option<i64>,
}

impl Default for SupporterState {
    fn default() -> Self {
        Self {
            schema_version: SUPPORTER_STATE_SCHEMA_VERSION,
            first_value_at_unix: None,
            last_value_at_unix: None,
            last_prompt_at_unix: None,
            recent_prompt_unix: Vec::new(),
            meaningful_successes_since_prompt: 0,
            dismiss_count: 0,
            dismiss_until_unix: None,
            claimed_supported_until_unix: None,
            support_clicked_until_unix: None,
        }
    }
}

impl SupporterState {
    pub(crate) fn record_meaningful_success(&mut self, now_unix: i64) {
        let now_unix = now_unix.max(0);
        self.first_value_at_unix.get_or_insert(now_unix);
        self.last_value_at_unix = Some(now_unix);
        self.meaningful_successes_since_prompt =
            self.meaningful_successes_since_prompt.saturating_add(1);
    }

    pub(crate) fn can_prompt(&self, now_unix: i64) -> bool {
        let now_unix = now_unix.max(0);
        if self.first_value_at_unix.is_none() {
            return false;
        }

        if is_suppressed(self.dismiss_until_unix, now_unix)
            || is_suppressed(self.claimed_supported_until_unix, now_unix)
            || is_suppressed(self.support_clicked_until_unix, now_unix)
        {
            return false;
        }

        if self.recent_prompt_count(now_unix) >= MAX_PROMPTS_PER_WINDOW {
            return false;
        }

        let Some(last_prompt_at) = self.last_prompt_at_unix else {
            return true;
        };

        if !elapsed_at_least(now_unix, last_prompt_at, MIN_PROMPT_COOLDOWN_SECONDS) {
            return false;
        }

        self.meaningful_successes_since_prompt >= REQUIRED_SUCCESSES_AFTER_PROMPT
    }

    pub(crate) fn record_prompt_shown(&mut self, now_unix: i64) {
        let now_unix = now_unix.max(0);
        self.recent_prompt_unix
            .retain(|shown_at| is_within_window(now_unix, *shown_at, PROMPT_WINDOW_SECONDS));
        self.recent_prompt_unix.push(now_unix);
        self.last_prompt_at_unix = Some(now_unix);
        self.meaningful_successes_since_prompt = 0;
    }

    pub(crate) fn record_later(&mut self, now_unix: i64) {
        self.dismiss_count = self.dismiss_count.saturating_add(1);
        if self.dismiss_count >= 2 {
            self.dismiss_until_unix = Some(
                now_unix
                    .max(0)
                    .saturating_add(REPEAT_DISMISS_SUPPRESSION_SECONDS),
            );
        }
    }

    pub(crate) fn record_already_supported(&mut self, now_unix: i64) {
        self.claimed_supported_until_unix = Some(
            now_unix
                .max(0)
                .saturating_add(CLAIMED_SUPPORTED_SUPPRESSION_SECONDS),
        );
    }

    pub(crate) fn record_support_clicked(&mut self, now_unix: i64) {
        self.support_clicked_until_unix = Some(
            now_unix
                .max(0)
                .saturating_add(SUPPORT_CLICK_SUPPRESSION_SECONDS),
        );
    }

    pub(crate) fn to_json_string(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "first_value_at_unix": self.first_value_at_unix,
            "last_value_at_unix": self.last_value_at_unix,
            "last_prompt_at_unix": self.last_prompt_at_unix,
            "recent_prompt_unix": self.recent_prompt_unix,
            "meaningful_successes_since_prompt": self.meaningful_successes_since_prompt,
            "dismiss_count": self.dismiss_count,
            "dismiss_until_unix": self.dismiss_until_unix,
            "claimed_supported_until_unix": self.claimed_supported_until_unix,
            "support_clicked_until_unix": self.support_clicked_until_unix,
        })
        .to_string()
    }

    pub(crate) fn from_json_str(raw: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        let object = value.as_object()?;
        let schema_version = u8::try_from(object.get("schema_version")?.as_u64()?).ok()?;
        if schema_version != SUPPORTER_STATE_SCHEMA_VERSION {
            return None;
        }

        let recent_prompt_unix = object
            .get("recent_prompt_unix")?
            .as_array()?
            .iter()
            .map(|value| value.as_i64())
            .collect::<Option<Vec<_>>>()?;

        Some(Self {
            schema_version,
            first_value_at_unix: optional_i64(object.get("first_value_at_unix")?)?,
            last_value_at_unix: optional_i64(object.get("last_value_at_unix")?)?,
            last_prompt_at_unix: optional_i64(object.get("last_prompt_at_unix")?)?,
            recent_prompt_unix,
            meaningful_successes_since_prompt: u16::try_from(
                object.get("meaningful_successes_since_prompt")?.as_u64()?,
            )
            .ok()?,
            dismiss_count: u16::try_from(object.get("dismiss_count")?.as_u64()?).ok()?,
            dismiss_until_unix: optional_i64(object.get("dismiss_until_unix")?)?,
            claimed_supported_until_unix: optional_i64(
                object.get("claimed_supported_until_unix")?,
            )?,
            support_clicked_until_unix: optional_i64(object.get("support_clicked_until_unix")?)?,
        })
    }

    fn recent_prompt_count(&self, now_unix: i64) -> usize {
        self.recent_prompt_unix
            .iter()
            .filter(|shown_at| is_within_window(now_unix, **shown_at, PROMPT_WINDOW_SECONDS))
            .count()
    }
}

fn optional_i64(value: &serde_json::Value) -> Option<Option<i64>> {
    if value.is_null() {
        Some(None)
    } else {
        value.as_i64().map(Some)
    }
}

fn is_suppressed(until_unix: Option<i64>, now_unix: i64) -> bool {
    until_unix.is_some_and(|until| now_unix < until)
}

fn elapsed_at_least(now_unix: i64, then_unix: i64, duration_seconds: i64) -> bool {
    now_unix >= then_unix && now_unix - then_unix >= duration_seconds
}

fn is_within_window(now_unix: i64, event_unix: i64, window_seconds: i64) -> bool {
    event_unix > now_unix || now_unix - event_unix < window_seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(status: OpenStatus) -> ValueEvent {
        ValueEvent::DocumentOpened {
            status,
            page_count: 14,
            text_searchable: true,
            initial_page: 0,
        }
    }

    #[test]
    fn first_meaningful_success_can_prompt_immediately() {
        let mut state = SupporterState::default();
        state.record_meaningful_success(1_000);

        assert!(state.can_prompt(1_000));
    }

    #[test]
    fn later_requires_both_time_and_three_new_successes() {
        let mut state = SupporterState::default();
        state.record_meaningful_success(1_000);
        state.record_prompt_shown(1_000);
        state.record_later(1_000);

        for offset in 1..=3 {
            state.record_meaningful_success(1_000 + offset);
        }

        assert!(!state.can_prompt(1_000 + MIN_PROMPT_COOLDOWN_SECONDS - 1));
        assert!(state.can_prompt(1_000 + MIN_PROMPT_COOLDOWN_SECONDS));
    }

    #[test]
    fn prompt_cap_is_a_real_rolling_window() {
        let mut state = SupporterState::default();
        state.record_meaningful_success(0);
        state.record_prompt_shown(0);
        state.record_later(0);

        for _ in 0..3 {
            state.record_meaningful_success(MIN_PROMPT_COOLDOWN_SECONDS);
        }
        state.record_prompt_shown(MIN_PROMPT_COOLDOWN_SECONDS);

        assert!(!state.can_prompt(MIN_PROMPT_COOLDOWN_SECONDS + MIN_PROMPT_COOLDOWN_SECONDS));

        let after_first_expires = PROMPT_WINDOW_SECONDS + 1;
        for _ in 0..3 {
            state.record_meaningful_success(after_first_expires);
        }
        assert!(state.can_prompt(after_first_expires));
    }

    #[test]
    fn second_dismissal_suppresses_for_forty_five_days() {
        let mut state = SupporterState::default();
        state.record_meaningful_success(0);
        state.record_prompt_shown(0);
        state.record_later(0);

        let second_prompt = MIN_PROMPT_COOLDOWN_SECONDS;
        for _ in 0..3 {
            state.record_meaningful_success(second_prompt);
        }
        assert!(state.can_prompt(second_prompt));
        state.record_prompt_shown(second_prompt);
        state.record_later(second_prompt);

        let before_end = second_prompt + REPEAT_DISMISS_SUPPRESSION_SECONDS - 1;
        assert!(!state.can_prompt(before_end));
    }

    #[test]
    fn already_supported_and_support_click_use_different_suppression() {
        let mut supported = SupporterState::default();
        supported.record_meaningful_success(100);
        supported.record_already_supported(100);
        assert!(!supported.can_prompt(100 + 179 * DAY_SECONDS));
        assert!(supported.can_prompt(100 + 180 * DAY_SECONDS));

        let mut clicked = SupporterState::default();
        clicked.record_meaningful_success(100);
        clicked.record_support_clicked(100);
        assert!(!clicked.can_prompt(100 + 29 * DAY_SECONDS));
        assert!(clicked.can_prompt(100 + 30 * DAY_SECONDS));
    }

    #[test]
    fn clock_rollback_is_conservatively_suppressed_after_prompt() {
        let mut state = SupporterState::default();
        state.record_meaningful_success(10_000);
        state.record_prompt_shown(10_000);

        for _ in 0..3 {
            state.record_meaningful_success(9_000);
        }

        assert!(!state.can_prompt(9_000));
    }

    #[test]
    fn supporter_state_round_trips_without_document_identity() {
        let mut state = SupporterState::default();
        state.record_meaningful_success(10);
        state.record_prompt_shown(20);
        state.record_later(20);

        let encoded = state.to_json_string();
        let decoded = SupporterState::from_json_str(&encoded).expect("state must decode");

        assert_eq!(decoded, state);

        let value: serde_json::Value =
            serde_json::from_str(&encoded).expect("serialized state must be valid JSON");
        let keys = value
            .as_object()
            .expect("state JSON must be an object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();

        let expected = [
            "schema_version",
            "first_value_at_unix",
            "last_value_at_unix",
            "last_prompt_at_unix",
            "recent_prompt_unix",
            "meaningful_successes_since_prompt",
            "dismiss_count",
            "dismiss_until_unix",
            "claimed_supported_until_unix",
            "support_clicked_until_unix",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(keys, expected);
        assert!(!encoded.contains("filename"));
        assert!(!encoded.contains("path"));
        assert!(!encoded.contains("hash"));
        assert!(!encoded.contains("text"));
    }

    #[test]
    fn unknown_or_malformed_persisted_state_fails_to_none() {
        assert!(SupporterState::from_json_str("{").is_none());

        let future = serde_json::json!({
            "schema_version": SUPPORTER_STATE_SCHEMA_VERSION + 1,
            "first_value_at_unix": null,
            "last_value_at_unix": null,
            "last_prompt_at_unix": null,
            "recent_prompt_unix": [],
            "meaningful_successes_since_prompt": 0,
            "dismiss_count": 0,
            "dismiss_until_unix": null,
            "claimed_supported_until_unix": null,
            "support_clicked_until_unix": null,
        });
        assert!(SupporterState::from_json_str(&future.to_string()).is_none());
    }

    #[test]
    fn opening_a_supported_document_is_not_value_by_itself() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));

        assert!(!tracker.is_eligible());
        assert_eq!(tracker.receipt(), None);
    }

    #[test]
    fn opening_a_partial_document_can_still_lead_to_value() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Partial));
        tracker.observe(ValueEvent::SearchMatchCopied);

        assert_eq!(
            tracker.receipt(),
            Some(ValueReceipt {
                page_count: 14,
                kind: ValueReceiptKind::TextCopied,
            })
        );
    }

    #[test]
    fn unsupported_document_never_becomes_eligible() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Unsupported));
        tracker.observe(ValueEvent::SearchResultSelected { match_count: 3 });
        tracker.observe(ValueEvent::SearchMatchCopied);
        tracker.observe(ValueEvent::PageNavigated { page_index: 1 });
        tracker.observe(ValueEvent::PageNavigated { page_index: 2 });

        assert!(!tracker.is_eligible());
    }

    #[test]
    fn first_page_change_alone_is_not_enough() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::PageNavigated { page_index: 1 });

        assert!(!tracker.is_eligible());
    }

    #[test]
    fn second_navigation_after_leaving_initial_page_is_reading_value() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::PageNavigated { page_index: 1 });
        tracker.observe(ValueEvent::PageNavigated { page_index: 2 });

        assert_eq!(
            tracker.receipt(),
            Some(ValueReceipt {
                page_count: 14,
                kind: ValueReceiptKind::Reading {
                    text_searchable: true,
                },
            })
        );
    }

    #[test]
    fn returning_to_initial_page_counts_as_further_deliberate_navigation() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::PageNavigated { page_index: 1 });
        tracker.observe(ValueEvent::PageNavigated { page_index: 0 });

        assert!(tracker.is_eligible());
    }

    #[test]
    fn selecting_a_real_search_result_is_immediate_value() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::SearchResultSelected { match_count: 7 });

        assert_eq!(
            tracker.receipt(),
            Some(ValueReceipt {
                page_count: 14,
                kind: ValueReceiptKind::SearchMatches { match_count: 7 },
            })
        );
    }

    #[test]
    fn zero_match_search_does_not_create_value() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::SearchResultSelected { match_count: 0 });

        assert!(!tracker.is_eligible());
    }

    #[test]
    fn copying_story_text_is_immediate_value() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::FullStoryCopied);

        assert_eq!(
            tracker.receipt(),
            Some(ValueReceipt {
                page_count: 14,
                kind: ValueReceiptKind::TextCopied,
            })
        );
    }

    #[test]
    fn first_value_receipt_is_stable_until_document_changes() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::SearchResultSelected { match_count: 7 });
        tracker.observe(ValueEvent::FullStoryCopied);

        assert_eq!(
            tracker.receipt(),
            Some(ValueReceipt {
                page_count: 14,
                kind: ValueReceiptKind::SearchMatches { match_count: 7 },
            })
        );
    }

    #[test]
    fn workflow_failure_clears_eligibility() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::SearchMatchCopied);
        assert!(tracker.is_eligible());

        tracker.observe(ValueEvent::WorkflowFailed);

        assert!(!tracker.is_eligible());
        assert_eq!(tracker.receipt(), None);
    }

    #[test]
    fn opening_a_new_document_resets_previous_value() {
        let mut tracker = ValueTracker::default();
        tracker.observe(open(OpenStatus::Supported));
        tracker.observe(ValueEvent::FullStoryCopied);
        assert!(tracker.is_eligible());

        tracker.observe(ValueEvent::DocumentOpened {
            status: OpenStatus::Partial,
            page_count: 3,
            text_searchable: false,
            initial_page: 0,
        });

        assert!(!tracker.is_eligible());
    }
}
