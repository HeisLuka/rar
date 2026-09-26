use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    PayloadSwap,
    InstallerRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Prepared,
    OriginExited,
    PreviousRetained,
    CandidatePromotedUnconfirmed,
    CandidateConfirmed,
    RollbackStarted,
    PreviousRestored,
    RollbackConfirmed,
    RepairRequired,
}

impl UpdatePhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::CandidateConfirmed | Self::RollbackConfirmed | Self::RepairRequired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateJournal {
    pub schema_version: u32,
    pub generation: u64,
    pub attempt_id: String,
    pub product_id: String,
    pub architecture: String,
    pub channel: String,
    pub from_version: String,
    pub to_version: String,
    pub install_layout_epoch: u32,
    pub update_protocol_version: u32,
    pub update_mode: UpdateMode,
    pub previous_tree_sha256: String,
    pub candidate_tree_sha256: String,
    pub rollback_compatible: bool,
    pub state_schema: String,
    pub phase: UpdatePhase,
}

impl UpdateJournal {
    #[allow(clippy::too_many_arguments)]
    pub fn new_payload_swap(
        product_id: impl Into<String>,
        architecture: impl Into<String>,
        channel: impl Into<String>,
        from_version: impl Into<String>,
        to_version: impl Into<String>,
        install_layout_epoch: u32,
        update_protocol_version: u32,
        previous_tree_sha256: impl Into<String>,
        candidate_tree_sha256: impl Into<String>,
        rollback_compatible: bool,
        state_schema: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            generation: 0,
            attempt_id: Uuid::now_v7().to_string(),
            product_id: product_id.into(),
            architecture: architecture.into(),
            channel: channel.into(),
            from_version: from_version.into(),
            to_version: to_version.into(),
            install_layout_epoch,
            update_protocol_version,
            update_mode: UpdateMode::PayloadSwap,
            previous_tree_sha256: previous_tree_sha256.into(),
            candidate_tree_sha256: candidate_tree_sha256.into(),
            rollback_compatible,
            state_schema: state_schema.into(),
            phase: UpdatePhase::Prepared,
        }
    }

    pub fn advance(&mut self, phase: UpdatePhase) {
        self.phase = phase;
    }
}
