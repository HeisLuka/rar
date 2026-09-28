#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateChannel { Stable, Beta }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateUiState {
    Idle,
    Checking,
    UpToDate { version: String },
    UpdateAvailable { version: String },
    Downloading { version: String, received: u64, total: Option<u64> },
    ReadyToRestart { version: String },
    Applying { version: String },
    Updated { version: String },
    RolledBack { restored_version: String },
    ErrorRetryable { message: String },
    RepairRequired { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommand {
    CheckMetadata { channel: UpdateChannel },
    Download { version: String },
    RestartAndApply { version: String },
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateUiModel {
    pub state: UpdateUiState,
    pub channel: UpdateChannel,
    pub automatic_checks: bool,
    pub close_is_admissible: bool,
}

impl Default for UpdateUiModel {
    fn default() -> Self {
        Self {
            state: UpdateUiState::Idle,
            channel: UpdateChannel::Stable,
            automatic_checks: false,
            close_is_admissible: true,
        }
    }
}

impl UpdateUiModel {
    pub fn manual_check(&mut self) -> UpdateCommand {
        self.state = UpdateUiState::Checking;
        UpdateCommand::CheckMetadata { channel: self.channel }
    }

    pub fn background_tick(&self) -> Option<UpdateCommand> {
        self.automatic_checks.then_some(UpdateCommand::CheckMetadata { channel: self.channel })
    }

    pub fn set_automatic_checks(&mut self, enabled: bool) {
        self.automatic_checks = enabled;
    }

    pub fn set_channel(&mut self, channel: UpdateChannel) {
        self.channel = channel;
    }

    pub fn metadata_result(&mut self, installed: &str, available: Option<&str>) {
        self.state = match available {
            Some(v) if v != installed => UpdateUiState::UpdateAvailable { version: v.into() },
            _ => UpdateUiState::UpToDate { version: installed.into() },
        };
    }

    pub fn check_failed(&mut self, message: impl Into<String>) {
        self.state = UpdateUiState::ErrorRetryable { message: message.into() };
    }

    pub fn trust_failed(&mut self) {
        self.state = UpdateUiState::ErrorRetryable {
            message: "Chaptera couldn't verify this update, so it wasn't installed.".into(),
        };
    }

    pub fn request_download(&self) -> Option<UpdateCommand> {
        match &self.state {
            UpdateUiState::UpdateAvailable { version } => Some(UpdateCommand::Download { version: version.clone() }),
            _ => None,
        }
    }

    pub fn download_ready(&mut self, version: impl Into<String>) {
        self.state = UpdateUiState::ReadyToRestart { version: version.into() };
    }

    pub fn request_restart_and_apply(&self) -> Option<UpdateCommand> {
        if !self.close_is_admissible { return None; }
        match &self.state {
            UpdateUiState::ReadyToRestart { version } => Some(UpdateCommand::RestartAndApply { version: version.clone() }),
            _ => None,
        }
    }

    pub fn rolled_back(&mut self, restored_version: impl Into<String>) {
        self.state = UpdateUiState::RolledBack { restored_version: restored_version.into() };
    }

    pub fn repair_required(&mut self, message: impl Into<String>) {
        self.state = UpdateUiState::RepairRequired { message: message.into() };
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
    fn automatic_checks_require_explicit_opt_in_and_metadata_only() {
        let mut model = UpdateUiModel::default();
        model.set_automatic_checks(true);
        assert_eq!(model.background_tick(), Some(UpdateCommand::CheckMetadata { channel: UpdateChannel::Stable }));
    }

    #[test]
    fn offline_failure_is_not_up_to_date() {
        let mut model = UpdateUiModel::default();
        model.manual_check();
        model.check_failed("Cannot check for updates right now");
        assert!(matches!(model.state, UpdateUiState::ErrorRetryable { .. }));
    }

    #[test]
    fn restart_requires_ready_state_and_admissible_close() {
        let mut model = UpdateUiModel::default();
        model.download_ready("0.2.0");
        model.close_is_admissible = false;
        assert_eq!(model.request_restart_and_apply(), None);
        model.close_is_admissible = true;
        assert_eq!(model.request_restart_and_apply(), Some(UpdateCommand::RestartAndApply { version: "0.2.0".into() }));
    }

    #[test]
    fn beta_to_stable_selection_never_emits_downgrade() {
        let mut model = UpdateUiModel::default();
        model.set_channel(UpdateChannel::Beta);
        model.metadata_result("0.3.0-beta.2", Some("0.3.0-beta.2"));
        model.set_channel(UpdateChannel::Stable);
        assert_eq!(model.request_download(), None);
        assert_eq!(model.request_restart_and_apply(), None);
    }

    #[test]
    fn rollback_and_repair_are_not_reported_as_success() {
        let mut model = UpdateUiModel::default();
        model.rolled_back("0.1.9");
        assert!(matches!(model.state, UpdateUiState::RolledBack { .. }));
        model.repair_required("both candidate and predecessor failed");
        assert!(matches!(model.state, UpdateUiState::RepairRequired { .. }));
        assert_eq!(model.request_repair(), Some(UpdateCommand::Repair));
    }
}
