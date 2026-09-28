#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateChannel {
    Stable,
    Beta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataOutcome {
    UpToDate {
        installed_version: String,
    },
    UpdateAvailable {
        installed_version: String,
        target_version: String,
    },
    ChannelHasNotCaughtUp {
        installed_version: String,
        channel_version: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateUiState {
    Idle,
    Checking,
    UpToDate {
        version: String,
    },
    UpdateAvailable {
        version: String,
    },
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    ReadyToRestart {
        version: String,
    },
    Applying {
        version: String,
    },
    Updated {
        version: String,
    },
    RolledBack {
        restored_version: String,
    },
    InsufficientSpace {
        required_bytes: u64,
        available_bytes: u64,
    },
    ChannelWaitingForCatchUp {
        installed_version: String,
        channel: UpdateChannel,
        channel_version: Option<String>,
    },
    ErrorRetryable {
        message: String,
    },
    RepairRequired {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommand {
    CheckMetadata {
        channel: UpdateChannel,
    },
    Download {
        version: String,
    },
    RestartAndApply {
        version: String,
    },
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelChangeDecision {
    NoChange,
    Applied(UpdateChannel),
    ConfirmationRequired(UpdateChannel),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateUiModel {
    pub state: UpdateUiState,
    pub channel: UpdateChannel,
    pub automatic_checks: bool,
    pub close_is_admissible: bool,
    pending_channel_confirmation: Option<UpdateChannel>,
    last_notified_version: Option<String>,
}

impl Default for UpdateUiModel {
    fn default() -> Self {
        Self {
            state: UpdateUiState::Idle,
            channel: UpdateChannel::Stable,
            automatic_checks: false,
            close_is_admissible: true,
            pending_channel_confirmation: None,
            last_notified_version: None,
        }
    }
}

impl UpdateUiModel {
    pub fn manual_check(&mut self) -> UpdateCommand {
        self.state = UpdateUiState::Checking;
        UpdateCommand::CheckMetadata {
            channel: self.channel,
        }
    }

    pub fn background_tick(&self) -> Option<UpdateCommand> {
        self.automatic_checks.then_some(UpdateCommand::CheckMetadata {
            channel: self.channel,
        })
    }

    pub fn set_automatic_checks(&mut self, enabled: bool) {
        self.automatic_checks = enabled;
    }

    pub fn request_channel_change(&mut self, target: UpdateChannel) -> ChannelChangeDecision {
        if target == self.channel {
            self.pending_channel_confirmation = None;
            return ChannelChangeDecision::NoChange;
        }

        if self.channel == UpdateChannel::Stable && target == UpdateChannel::Beta {
            self.pending_channel_confirmation = Some(target);
            return ChannelChangeDecision::ConfirmationRequired(target);
        }

        self.channel = target;
        self.pending_channel_confirmation = None;
        ChannelChangeDecision::Applied(target)
    }

    pub fn confirm_channel_change(&mut self, target: UpdateChannel) -> ChannelChangeDecision {
        if self.pending_channel_confirmation == Some(target) {
            self.channel = target;
            self.pending_channel_confirmation = None;
            ChannelChangeDecision::Applied(target)
        } else {
            ChannelChangeDecision::NoChange
        }
    }

    pub fn metadata_result(&mut self, outcome: MetadataOutcome) {
        self.state = match outcome {
            MetadataOutcome::UpToDate { installed_version } => UpdateUiState::UpToDate {
                version: installed_version,
            },
            MetadataOutcome::UpdateAvailable {
                target_version, ..
            } => UpdateUiState::UpdateAvailable {
                version: target_version,
            },
            MetadataOutcome::ChannelHasNotCaughtUp {
                installed_version,
                channel_version,
            } => UpdateUiState::ChannelWaitingForCatchUp {
                installed_version,
                channel: self.channel,
                channel_version,
            },
        };
    }

    pub fn should_surface_update_notification(&mut self) -> bool {
        let UpdateUiState::UpdateAvailable { version } = &self.state else {
            return false;
        };
        if self.last_notified_version.as_deref() == Some(version.as_str()) {
            return false;
        }
        self.last_notified_version = Some(version.clone());
        true
    }

    pub fn check_failed(&mut self, message: impl Into<String>) {
        self.state = UpdateUiState::ErrorRetryable {
            message: message.into(),
        };
    }

    pub fn trust_failed(&mut self) {
        self.state = UpdateUiState::ErrorRetryable {
            message: "Chaptera couldn't verify this update, so it wasn't installed.".into(),
        };
    }

    pub fn request_download(&self) -> Option<UpdateCommand> {
        match &self.state {
            UpdateUiState::UpdateAvailable { version } => Some(UpdateCommand::Download {
                version: version.clone(),
            }),
            _ => None,
        }
    }

    pub fn download_progress(
        &mut self,
        version: impl Into<String>,
        received: u64,
        total: Option<u64>,
    ) {
        self.state = UpdateUiState::Downloading {
            version: version.into(),
            received,
            total,
        };
    }

    pub fn insufficient_space(&mut self, required_bytes: u64, available_bytes: u64) {
        self.state = UpdateUiState::InsufficientSpace {
            required_bytes,
            available_bytes,
        };
    }

    pub fn download_ready(&mut self, version: impl Into<String>) {
        self.state = UpdateUiState::ReadyToRestart {
            version: version.into(),
        };
    }

    pub fn request_restart_and_apply(&self) -> Option<UpdateCommand> {
        if !self.close_is_admissible {
            return None;
        }
        match &self.state {
            UpdateUiState::ReadyToRestart { version } => Some(UpdateCommand::RestartAndApply {
                version: version.clone(),
            }),
            _ => None,
        }
    }

    pub fn applying(&mut self, version: impl Into<String>) {
        self.state = UpdateUiState::Applying {
            version: version.into(),
        };
    }

    pub fn updated(&mut self, version: impl Into<String>) {
        self.state = UpdateUiState::Updated {
            version: version.into(),
        };
    }

    pub fn rolled_back(&mut self, restored_version: impl Into<String>) {
        self.state = UpdateUiState::RolledBack {
            restored_version: restored_version.into(),
        };
    }

    pub fn repair_required(&mut self, message: impl Into<String>) {
        self.state = UpdateUiState::RepairRequired {
            message: message.into(),
        };
    }

    pub fn request_repair(&self) -> Option<UpdateCommand> {
        matches!(self.state, UpdateUiState::RepairRequired { .. }).then_some(UpdateCommand::Repair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_launch_emits_no_network_command() {
        let model = UpdateUiModel::default();
        assert_eq!(model.background_tick(), None);
        assert_eq!(model.channel, UpdateChannel::Stable);
    }

    #[test]
    fn automatic_checks_require_explicit_opt_in_and_are_metadata_only() {
        let mut model = UpdateUiModel::default();
        model.set_automatic_checks(true);
        assert_eq!(
            model.background_tick(),
            Some(UpdateCommand::CheckMetadata {
                channel: UpdateChannel::Stable
            })
        );
    }

    #[test]
    fn offline_failure_is_not_up_to_date() {
        let mut model = UpdateUiModel::default();
        model.manual_check();
        model.check_failed("Cannot check for updates right now");
        assert!(matches!(
            model.state,
            UpdateUiState::ErrorRetryable { .. }
        ));
    }

    #[test]
    fn untrusted_update_is_not_actionable() {
        let mut model = UpdateUiModel::default();
        model.trust_failed();
        assert_eq!(model.request_download(), None);
        assert_eq!(model.request_restart_and_apply(), None);
    }

    #[test]
    fn restart_requires_ready_state_and_admissible_close() {
        let mut model = UpdateUiModel::default();
        model.download_ready("0.2.0");
        model.close_is_admissible = false;
        assert_eq!(model.request_restart_and_apply(), None);
        model.close_is_admissible = true;
        assert_eq!(
            model.request_restart_and_apply(),
            Some(UpdateCommand::RestartAndApply {
                version: "0.2.0".into()
            })
        );
    }

    #[test]
    fn stable_to_beta_requires_explicit_confirmation() {
        let mut model = UpdateUiModel::default();
        assert_eq!(
            model.request_channel_change(UpdateChannel::Beta),
            ChannelChangeDecision::ConfirmationRequired(UpdateChannel::Beta)
        );
        assert_eq!(model.channel, UpdateChannel::Stable);
        assert_eq!(
            model.confirm_channel_change(UpdateChannel::Beta),
            ChannelChangeDecision::Applied(UpdateChannel::Beta)
        );
        assert_eq!(model.channel, UpdateChannel::Beta);
    }

    #[test]
    fn beta_to_stable_waits_when_provider_says_stable_has_not_caught_up() {
        let mut model = UpdateUiModel::default();
        model.request_channel_change(UpdateChannel::Beta);
        model.confirm_channel_change(UpdateChannel::Beta);

        assert_eq!(
            model.request_channel_change(UpdateChannel::Stable),
            ChannelChangeDecision::Applied(UpdateChannel::Stable)
        );
        model.metadata_result(MetadataOutcome::ChannelHasNotCaughtUp {
            installed_version: "0.3.0-beta.2".into(),
            channel_version: Some("0.2.9".into()),
        });

        assert!(matches!(
            model.state,
            UpdateUiState::ChannelWaitingForCatchUp {
                channel: UpdateChannel::Stable,
                ..
            }
        ));
        assert_eq!(model.request_download(), None);
        assert_eq!(model.request_restart_and_apply(), None);
    }

    #[test]
    fn repeated_same_version_notification_is_bounded() {
        let mut model = UpdateUiModel::default();
        model.metadata_result(MetadataOutcome::UpdateAvailable {
            installed_version: "0.1.0".into(),
            target_version: "0.2.0".into(),
        });
        assert!(model.should_surface_update_notification());
        assert!(!model.should_surface_update_notification());

        model.metadata_result(MetadataOutcome::UpdateAvailable {
            installed_version: "0.1.0".into(),
            target_version: "0.2.1".into(),
        });
        assert!(model.should_surface_update_notification());
    }

    #[test]
    fn insufficient_space_is_retryable_but_cannot_switch_or_restart() {
        let mut model = UpdateUiModel::default();
        model.insufficient_space(4096, 1024);
        assert_eq!(model.request_download(), None);
        assert_eq!(model.request_restart_and_apply(), None);
        assert!(matches!(
            model.state,
            UpdateUiState::InsufficientSpace {
                required_bytes: 4096,
                available_bytes: 1024
            }
        ));
    }

    #[test]
    fn progress_and_success_are_explicit_states() {
        let mut model = UpdateUiModel::default();
        model.download_progress("0.2.0", 512, Some(1024));
        assert!(matches!(
            model.state,
            UpdateUiState::Downloading {
                received: 512,
                total: Some(1024),
                ..
            }
        ));
        model.applying("0.2.0");
        assert!(matches!(model.state, UpdateUiState::Applying { .. }));
        model.updated("0.2.0");
        assert_eq!(
            model.state,
            UpdateUiState::Updated {
                version: "0.2.0".into()
            }
        );
    }

    #[test]
    fn rollback_and_repair_are_not_reported_as_success() {
        let mut model = UpdateUiModel::default();
        model.rolled_back("0.1.9");
        assert!(matches!(
            model.state,
            UpdateUiState::RolledBack { .. }
        ));
        model.repair_required("both candidate and predecessor failed");
        assert!(matches!(
            model.state,
            UpdateUiState::RepairRequired { .. }
        ));
        assert_eq!(model.request_repair(), Some(UpdateCommand::Repair));
    }
}
