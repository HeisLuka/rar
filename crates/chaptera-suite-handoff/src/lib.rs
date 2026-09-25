use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const PACKET_VERSION: &str = "chaptera.suite-handoff.v1";
pub const ACCEPTANCE_VERSION: &str = "chaptera.suite-handoff-acceptance.v1";
pub const READER_PRODUCT_ID: &str = "chaptera.reader";
pub const EDITOR_PRODUCT_ID: &str = "chaptera.editor";
pub const RESCUE_PRODUCT_ID: &str = "chaptera.rescue";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffPacket {
    pub protocol_version: String,
    pub sender_product_id: String,
    pub target_product_id: String,
    pub requested_job: String,
    pub source: HandoffSource,
    pub context: HandoffContext,
    pub provenance: HandoffProvenance,
    pub consent: HandoffConsent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffSource {
    pub path: String,
    pub sha256: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffContext {
    pub capability: String,
    pub loss_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffProvenance {
    pub source_identity_verified: bool,
    pub mutable_document_state_included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffConsent {
    pub local_file_handoff: bool,
    pub user_initiated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffAcceptance {
    pub protocol_version: String,
    pub sender_product_id: String,
    pub receiver_product_id: String,
    pub requested_job: String,
    pub handoff_packet_sha256: String,
    pub source_sha256: String,
    pub source_unchanged: bool,
    pub receiver_capability_admitted: bool,
    pub mutable_document_state_received: bool,
    pub source_path_serialized: bool,
}

#[derive(Debug, Clone)]
pub struct ValidatedHandoff {
    packet: HandoffPacket,
    packet_sha256: String,
    source_path: PathBuf,
    source_before_sha256: String,
}

impl ValidatedHandoff {
    pub fn packet(&self) -> &HandoffPacket {
        &self.packet
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn require_pub_path(path: &Path) -> Result<(), String> {
    let is_pub = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        == Some("pub".to_owned());
    if !is_pub {
        return Err("suite handoff source must be a .pub file".to_owned());
    }
    Ok(())
}

fn route_for_target(
    target_product_id: &str,
    reader_supported: bool,
    rescue_eligible: bool,
) -> Result<(&'static str, &'static str, &'static str), String> {
    match (target_product_id, reader_supported, rescue_eligible) {
        (EDITOR_PRODUCT_ID, true, _) => {
            Ok(("edit_supported_pub", "reader_supported", "none_observed"))
        }
        (EDITOR_PRODUCT_ID, false, _) => {
            Err("Reader may hand off to Editor only after the Reader open gate succeeds".to_owned())
        }
        (RESCUE_PRODUCT_ID, false, true) => Ok((
            "diagnose_or_recover",
            "reader_failure_recovery_eligible",
            "unknown",
        )),
        (RESCUE_PRODUCT_ID, true, _) => Err(
            "Reader may not hand a successfully opened source to Rescue".to_owned(),
        ),
        (RESCUE_PRODUCT_ID, false, false) => Err(
            "Reader failure is not recovery-eligible; healthy/unknown unsupported input must not be promoted to Rescue".to_owned(),
        ),
        _ => Err(format!(
            "unsupported suite handoff target: {target_product_id}"
        )),
    }
}

pub fn create_reader_handoff(
    source_path: &Path,
    target_product_id: &str,
    reader_supported: bool,
    rescue_eligible: bool,
) -> Result<HandoffPacket, String> {
    require_pub_path(source_path)?;
    let canonical = fs::canonicalize(source_path)
        .map_err(|error| format!("canonicalize {}: {error}", source_path.display()))?;
    let source_sha256 = sha256_file(&canonical)?;
    let (requested_job, capability, loss_state) =
        route_for_target(target_product_id, reader_supported, rescue_eligible)?;

    Ok(HandoffPacket {
        protocol_version: PACKET_VERSION.to_owned(),
        sender_product_id: READER_PRODUCT_ID.to_owned(),
        target_product_id: target_product_id.to_owned(),
        requested_job: requested_job.to_owned(),
        source: HandoffSource {
            path: canonical.display().to_string(),
            sha256: source_sha256,
            kind: "pub".to_owned(),
        },
        context: HandoffContext {
            capability: capability.to_owned(),
            loss_state: loss_state.to_owned(),
        },
        provenance: HandoffProvenance {
            source_identity_verified: true,
            mutable_document_state_included: false,
        },
        consent: HandoffConsent {
            local_file_handoff: true,
            user_initiated: true,
        },
    })
}

pub fn validate_packet(packet: &HandoffPacket) -> Result<(), String> {
    if packet.protocol_version != PACKET_VERSION {
        return Err(format!("unsupported suite handoff version: {}", packet.protocol_version));
    }
    if packet.sender_product_id != READER_PRODUCT_ID {
        return Err("current V1 sender must be chaptera.reader".to_owned());
    }
    if packet.source.kind != "pub" || !lower_sha256(&packet.source.sha256) {
        return Err("handoff source identity is invalid".to_owned());
    }
    if !packet.provenance.source_identity_verified
        || packet.provenance.mutable_document_state_included
    {
        return Err("handoff must carry verified source identity and no mutable document state".to_owned());
    }
    if !packet.consent.local_file_handoff || !packet.consent.user_initiated {
        return Err("handoff requires explicit local user intent".to_owned());
    }

    match packet.target_product_id.as_str() {
        EDITOR_PRODUCT_ID => {
            if packet.requested_job != "edit_supported_pub"
                || packet.context.capability != "reader_supported"
                || packet.context.loss_state != "none_observed"
            {
                return Err("Reader→Editor handoff context is inconsistent".to_owned());
            }
        }
        RESCUE_PRODUCT_ID => {
            if packet.requested_job != "diagnose_or_recover"
                || packet.context.capability != "reader_failure_recovery_eligible"
                || packet.context.loss_state != "unknown"
            {
                return Err("Reader→Rescue handoff context is inconsistent".to_owned());
            }
        }
        other => return Err(format!("unsupported suite handoff receiver: {other}")),
    }
    Ok(())
}

pub fn write_packet(packet: &HandoffPacket, output: &Path) -> Result<(), String> {
    validate_packet(packet)?;
    if Path::new(&packet.source.path) == output {
        return Err("handoff packet must not overwrite the source PUB".to_owned());
    }
    if let Some(parent) = output.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(packet)
        .map_err(|error| format!("serialize suite handoff packet: {error}"))?;
    fs::write(output, [bytes.as_slice(), b"\n"].concat())
        .map_err(|error| format!("write {}: {error}", output.display()))
}

pub fn load_for_receiver(
    packet_path: &Path,
    expected_receiver: &str,
) -> Result<ValidatedHandoff, String> {
    let packet_bytes =
        fs::read(packet_path).map_err(|error| format!("read {}: {error}", packet_path.display()))?;
    let packet: HandoffPacket = serde_json::from_slice(&packet_bytes)
        .map_err(|error| format!("parse suite handoff packet: {error}"))?;
    validate_packet(&packet)?;
    if packet.target_product_id != expected_receiver {
        return Err(format!(
            "handoff target {} does not match receiver {expected_receiver}",
            packet.target_product_id
        ));
    }
    let packet_sha256 = {
        let mut digest = Sha256::new();
        digest.update(&packet_bytes);
        format!("{:x}", digest.finalize())
    };
    let source_path = PathBuf::from(&packet.source.path);
    require_pub_path(&source_path)?;
    let source_before_sha256 = sha256_file(&source_path)?;
    if source_before_sha256 != packet.source.sha256 {
        return Err("handoff source identity changed after sender admission".to_owned());
    }
    Ok(ValidatedHandoff {
        packet,
        packet_sha256,
        source_path,
        source_before_sha256,
    })
}

pub fn finish_acceptance(
    validated: ValidatedHandoff,
    receiver_capability_admitted: bool,
) -> Result<HandoffAcceptance, String> {
    if !receiver_capability_admitted {
        return Err("receiver capability gate declined this handoff".to_owned());
    }
    let source_after_sha256 = sha256_file(&validated.source_path)?;
    if source_after_sha256 != validated.source_before_sha256 {
        return Err("handoff source identity changed during receiver admission".to_owned());
    }
    Ok(HandoffAcceptance {
        protocol_version: ACCEPTANCE_VERSION.to_owned(),
        sender_product_id: validated.packet.sender_product_id,
        receiver_product_id: validated.packet.target_product_id,
        requested_job: validated.packet.requested_job,
        handoff_packet_sha256: validated.packet_sha256,
        source_sha256: validated.source_before_sha256,
        source_unchanged: true,
        receiver_capability_admitted: true,
        mutable_document_state_received: false,
        source_path_serialized: false,
    })
}

pub fn write_acceptance(receipt: &HandoffAcceptance, output: &Path) -> Result<(), String> {
    if receipt.protocol_version != ACCEPTANCE_VERSION
        || !receipt.source_unchanged
        || !receipt.receiver_capability_admitted
        || receipt.mutable_document_state_received
        || receipt.source_path_serialized
        || !lower_sha256(&receipt.handoff_packet_sha256)
        || !lower_sha256(&receipt.source_sha256)
    {
        return Err("invalid suite handoff acceptance receipt".to_owned());
    }
    if let Some(parent) = output.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("serialize suite handoff acceptance: {error}"))?;
    fs::write(output, [bytes.as_slice(), b"\n"].concat())
        .map_err(|error| format!("write {}: {error}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pub(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "chaptera-suite-handoff-{name}-{}.pub",
            std::process::id()
        ));
        fs::write(&path, b"synthetic handoff source").expect("write temp source");
        path
    }

    #[test]
    fn reader_editor_packet_contains_identity_but_no_mutable_state() {
        let source = temp_pub("editor");
        let packet = create_reader_handoff(&source, EDITOR_PRODUCT_ID, true, false).expect("packet");
        assert_eq!(packet.requested_job, "edit_supported_pub");
        assert!(!packet.provenance.mutable_document_state_included);
        assert!(packet.provenance.source_identity_verified);
        fs::remove_file(source).ok();
    }

    #[test]
    fn supported_reader_source_is_not_routed_to_rescue() {
        let source = temp_pub("no-rescue");
        let error = create_reader_handoff(&source, RESCUE_PRODUCT_ID, true, false)
            .expect_err("supported source must not route to Rescue");
        assert!(error.contains("Rescue"));
        fs::remove_file(source).ok();
    }

    #[test]
    fn unsupported_but_not_recovery_eligible_does_not_route_to_rescue() {
        let source = temp_pub("unsupported-not-rescue");
        let error = create_reader_handoff(&source, RESCUE_PRODUCT_ID, false, false)
            .expect_err("unsupported input must not automatically become Rescue");
        assert!(error.contains("not recovery-eligible"));
        fs::remove_file(source).ok();
    }

    #[test]
    fn damaged_recovery_eligible_source_routes_to_rescue() {
        let source = temp_pub("damaged-rescue");
        let packet = create_reader_handoff(&source, RESCUE_PRODUCT_ID, false, true)
            .expect("recovery-eligible failure routes to Rescue");
        assert_eq!(packet.requested_job, "diagnose_or_recover");
        assert_eq!(
            packet.context.capability,
            "reader_failure_recovery_eligible"
        );
        fs::remove_file(source).ok();
    }

    #[test]
    fn receiver_detects_source_tampering() {
        let source = temp_pub("tamper");
        let packet = create_reader_handoff(&source, EDITOR_PRODUCT_ID, true, false).expect("packet");
        let packet_path = source.with_extension("handoff.json");
        write_packet(&packet, &packet_path).expect("write packet");
        fs::write(&source, b"changed").expect("tamper source");
        let error = load_for_receiver(&packet_path, EDITOR_PRODUCT_ID)
            .expect_err("tampered source must fail");
        assert!(error.contains("identity changed"));
        fs::remove_file(source).ok();
        fs::remove_file(packet_path).ok();
    }
}
