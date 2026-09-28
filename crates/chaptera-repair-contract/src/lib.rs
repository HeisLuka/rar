use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenInstallEvidence {
    pub candidate_tree_digest: String,
    pub predecessor_tree_digest: String,
    pub failure_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairTarget {
    pub product_id: String,
    pub architecture: String,
    pub channel: String,
    pub version: String,
    pub installer_sha256: String,
    pub installer_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairState {
    RepairRequired { evidence: BrokenInstallEvidence },
    Verifying { evidence: BrokenInstallEvidence, target: RepairTarget },
    Applying { evidence: BrokenInstallEvidence, target: RepairTarget },
    Repaired { version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairError {
    NotRepairRequired,
    WrongProduct,
    WrongArchitecture,
    WrongChannel,
    InvalidLength,
    InvalidDigest,
    InstallerLengthMismatch { expected: u64, actual: u64 },
    InstallerDigestMismatch,
    InstallerFailed(String),
    SelfCheckFailed(String),
}

pub struct RepairSession {
    state: RepairState,
}

impl RepairSession {
    pub fn required(evidence: BrokenInstallEvidence) -> Self {
        Self { state: RepairState::RepairRequired { evidence } }
    }

    pub fn state(&self) -> &RepairState { &self.state }

    pub fn begin_verified_target(
        &mut self,
        expected_product: &str,
        expected_architecture: &str,
        expected_channel: &str,
        target: RepairTarget,
    ) -> Result<(), RepairError> {
        let evidence = match &self.state {
            RepairState::RepairRequired { evidence } => evidence.clone(),
            _ => return Err(RepairError::NotRepairRequired),
        };
        if target.product_id != expected_product { return Err(RepairError::WrongProduct); }
        if target.architecture != expected_architecture { return Err(RepairError::WrongArchitecture); }
        if target.channel != expected_channel { return Err(RepairError::WrongChannel); }
        if target.installer_length == 0 { return Err(RepairError::InvalidLength); }
        if target.installer_sha256.len() != 64 || !target.installer_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RepairError::InvalidDigest);
        }
        self.state = RepairState::Verifying { evidence, target };
        Ok(())
    }

    pub fn verify_installer_bytes(&mut self, installer_bytes: &[u8]) -> Result<(), RepairError> {
        let (evidence, target) = match &self.state {
            RepairState::Verifying { evidence, target } => (evidence.clone(), target.clone()),
            _ => return Err(RepairError::NotRepairRequired),
        };

        let actual_length = installer_bytes.len() as u64;
        if actual_length != target.installer_length {
            self.state = RepairState::RepairRequired { evidence };
            return Err(RepairError::InstallerLengthMismatch {
                expected: target.installer_length,
                actual: actual_length,
            });
        }

        let actual_digest = format!("{:x}", Sha256::digest(installer_bytes));
        if !actual_digest.eq_ignore_ascii_case(&target.installer_sha256) {
            self.state = RepairState::RepairRequired { evidence };
            return Err(RepairError::InstallerDigestMismatch);
        }

        self.state = RepairState::Applying { evidence, target };
        Ok(())
    }

    pub fn installer_failed(&mut self, reason: impl Into<String>) -> RepairError {
        let reason = reason.into();
        let evidence = match &self.state {
            RepairState::Applying { evidence, .. } | RepairState::Verifying { evidence, .. } => evidence.clone(),
            RepairState::RepairRequired { evidence } => evidence.clone(),
            RepairState::Repaired { .. } => return RepairError::NotRepairRequired,
        };
        self.state = RepairState::RepairRequired { evidence };
        RepairError::InstallerFailed(reason)
    }

    pub fn self_check_failed(&mut self, code: impl Into<String>) -> RepairError {
        let code = code.into();
        let mut evidence = match &self.state {
            RepairState::Applying { evidence, .. } => evidence.clone(),
            RepairState::RepairRequired { evidence } => evidence.clone(),
            _ => return RepairError::NotRepairRequired,
        };
        evidence.failure_codes.push(code.clone());
        self.state = RepairState::RepairRequired { evidence };
        RepairError::SelfCheckFailed(code)
    }

    pub fn confirm_repaired(&mut self, installed_version: impl Into<String>) -> Result<(), RepairError> {
        if !matches!(self.state, RepairState::Applying { .. }) {
            return Err(RepairError::NotRepairRequired);
        }
        self.state = RepairState::Repaired { version: installed_version.into() };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLER: &[u8] = b"chaptera-test-installer-v1";

    fn evidence() -> BrokenInstallEvidence {
        BrokenInstallEvidence {
            candidate_tree_digest: "a".repeat(64),
            predecessor_tree_digest: "b".repeat(64),
            failure_codes: vec!["candidate_self_check_failed".into(), "predecessor_unconfirmed".into()],
        }
    }

    fn target() -> RepairTarget {
        RepairTarget {
            product_id: "chaptera.reader".into(),
            architecture: "windows-x86_64".into(),
            channel: "stable".into(),
            version: "0.2.0".into(),
            installer_sha256: format!("{:x}", Sha256::digest(INSTALLER)),
            installer_length: INSTALLER.len() as u64,
        }
    }

    #[test]
    fn repair_is_explicit_and_evidence_survives_failed_installer() {
        let original = evidence();
        let mut session = RepairSession::required(original.clone());
        session.begin_verified_target("chaptera.reader", "windows-x86_64", "stable", target()).unwrap();
        session.verify_installer_bytes(INSTALLER).unwrap();
        let err = session.installer_failed("installer exit 1603");
        assert_eq!(err, RepairError::InstallerFailed("installer exit 1603".into()));
        assert_eq!(session.state(), &RepairState::RepairRequired { evidence: original });
    }

    #[test]
    fn wrong_identity_fails_before_apply() {
        let mut session = RepairSession::required(evidence());
        let mut wrong = target();
        wrong.product_id = "other.product".into();
        assert_eq!(
            session.begin_verified_target("chaptera.reader", "windows-x86_64", "stable", wrong),
            Err(RepairError::WrongProduct)
        );
        assert!(matches!(session.state(), RepairState::RepairRequired { .. }));
    }

    #[test]
    fn installer_bytes_are_bound_to_authenticated_length_and_digest() {
        let original = evidence();
        let mut session = RepairSession::required(original.clone());
        session.begin_verified_target("chaptera.reader", "windows-x86_64", "stable", target()).unwrap();

        assert_eq!(
            session.verify_installer_bytes(b"tampered"),
            Err(RepairError::InstallerLengthMismatch {
                expected: INSTALLER.len() as u64,
                actual: 8,
            })
        );
        assert_eq!(session.state(), &RepairState::RepairRequired { evidence: original });

        let mut same_length_tamper = INSTALLER.to_vec();
        same_length_tamper[0] ^= 0x01;
        let original = evidence();
        let mut session = RepairSession::required(original.clone());
        session.begin_verified_target("chaptera.reader", "windows-x86_64", "stable", target()).unwrap();
        assert_eq!(
            session.verify_installer_bytes(&same_length_tamper),
            Err(RepairError::InstallerDigestMismatch)
        );
        assert_eq!(session.state(), &RepairState::RepairRequired { evidence: original });
    }

    #[test]
    fn repair_clears_required_only_after_verified_bytes_and_explicit_success() {
        let mut session = RepairSession::required(evidence());
        session.begin_verified_target("chaptera.reader", "windows-x86_64", "stable", target()).unwrap();
        session.verify_installer_bytes(INSTALLER).unwrap();
        assert!(matches!(session.state(), RepairState::Applying { .. }));
        session.confirm_repaired("0.2.0").unwrap();
        assert_eq!(session.state(), &RepairState::Repaired { version: "0.2.0".into() });
    }

    #[test]
    fn failed_post_install_self_check_returns_to_repair_required_with_evidence() {
        let mut session = RepairSession::required(evidence());
        session.begin_verified_target("chaptera.reader", "windows-x86_64", "stable", target()).unwrap();
        session.verify_installer_bytes(INSTALLER).unwrap();
        let err = session.self_check_failed("reader_smoke_failed");
        assert_eq!(err, RepairError::SelfCheckFailed("reader_smoke_failed".into()));
        match session.state() {
            RepairState::RepairRequired { evidence } => assert!(evidence.failure_codes.contains(&"reader_smoke_failed".into())),
            _ => panic!("must remain RepairRequired"),
        }
    }
}
