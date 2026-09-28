//! Platform-neutral Chaptera Reader boundary intended for mobile shells.
//!
//! This crate deliberately owns no Android, JNI, Kotlin, egui, filesystem, or
//! source-format semantics. A mobile shell supplies local PUB bytes plus a
//! bounded layout environment and receives Viewer/render-plan facts produced by
//! the same Reader pipeline used by the desktop product.

use anyhow::Result;
use chaptera_untrusted_pub_scan::{
    PubScanPolicyV1, PubScanStatusV1, inspect_pub_bytes_v1,
};

mod image_decode;
pub use image_decode::{MobileAdmittedImageV1, MobileImageDecodeError, decode_mobile_image_v1};
pub use chaptera_viewer_render_plan::{PageRenderPlanV1, RenderPlanErrorV1};
use chaptera_viewer_render_plan::build_page_render_plan_v1;
pub use pub_model::ResourceId;
pub use pub_viewer::{
    BoundedLayoutEnvironment, ViewerDiagnostic, ViewerFidelityStatus, ViewerTextMatch,
    local_failure_diagnostic_json,
};
use pub_viewer::{
    ViewerGeometryDocument, open_mature_0x2c_geometry, viewer_geometry_environment_v0_1,
};

pub const MOBILE_READER_CORE_SCHEMA_V1: &str = "chaptera.mobile-reader-core.v1";
pub const MOBILE_READER_MAX_FILE_BYTES_V1: u64 = 128 * 1024 * 1024;

const CFB_MAGIC_V1: &[u8; 8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
pub const MOBILE_ADMISSION_FAILURE_SCHEMA_V1: &str =
    "chaptera.mobile-reader-admission-failure.v1";


pub fn mobile_reader_admission_policy_v1() -> PubScanPolicyV1 {
    PubScanPolicyV1 {
        max_file_bytes: MOBILE_READER_MAX_FILE_BYTES_V1,
        ..PubScanPolicyV1::default()
    }
}

/// Applies the portable structural admission shared with hostile-PUB tooling.
///
/// This is an in-memory CFB/resource fence only. It deliberately does not claim
/// Linux seccomp, process isolation, filesystem confinement, or malware scan on
/// Android/iOS.
pub fn admit_mobile_pub_bytes_v1(bytes: &[u8]) -> Result<()> {
    admit_mobile_pub_bytes_with_policy_v1(bytes, mobile_reader_admission_policy_v1())
}

fn admit_mobile_pub_bytes_with_policy_v1(bytes: &[u8], policy: PubScanPolicyV1) -> Result<()> {
    let result = inspect_pub_bytes_v1(bytes, policy, false);
    match result.status {
        PubScanStatusV1::AcceptedCfb => Ok(()),
        PubScanStatusV1::ParseFailed => {
            Err(anyhow::anyhow!("mobile_reader_admission.parse_failed"))
        }
        PubScanStatusV1::RejectedByPolicy => {
            let event = result
                .security_event
                .as_deref()
                .unwrap_or("policy_rejected");
            Err(anyhow::anyhow!(
                "mobile_reader_admission.rejected_by_policy:{event}"
            ))
        }
    }

}

pub fn mobile_failure_diagnostic_json(bytes: &[u8]) -> Result<String> {
    mobile_failure_diagnostic_json_with_policy(bytes, mobile_reader_admission_policy_v1())
}

fn mobile_failure_diagnostic_json_with_policy(
    bytes: &[u8],
    policy: PubScanPolicyV1,
) -> Result<String> {
    let admission = inspect_pub_bytes_v1(bytes, policy, false);
    let bypass_shared_classifier = matches!(admission.status, PubScanStatusV1::RejectedByPolicy)
        || (matches!(admission.status, PubScanStatusV1::ParseFailed)
            && bytes.starts_with(CFB_MAGIC_V1));

    if bypass_shared_classifier {
        return serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": MOBILE_ADMISSION_FAILURE_SCHEMA_V1,
            "admission": {
                "status": match admission.status {
                    PubScanStatusV1::RejectedByPolicy => "rejected_by_policy",
                    PubScanStatusV1::ParseFailed => "parse_failed",
                    PubScanStatusV1::AcceptedCfb => "accepted_cfb",
                },
                "security_event": admission.security_event.as_deref().unwrap_or("none"),
            },
            "contains_document_bytes": false,
            "contains_recovered_document_text": false,
        }))
        .map_err(Into::into);
    }

    pub_viewer::local_failure_diagnostic_json(bytes)


/// Read-only, source-neutral document state for a mobile Reader shell.
///
/// The underlying Viewer document is intentionally private so platform code
/// cannot bypass the shared render-plan boundary and start depending on PUB
/// parser internals.
pub struct MobileReaderDocumentV1 {
    visual: ViewerGeometryDocument,
}

impl MobileReaderDocumentV1 {
    /// Opens immutable local PUB bytes through the canonical Viewer pipeline.
    ///
    /// The caller owns acquiring bytes from Android/iOS storage APIs. This
    /// function performs no network or filesystem I/O and does not mutate the
    /// supplied source buffer.
    pub fn open(bytes: &[u8], environment: BoundedLayoutEnvironment) -> Result<Self> {
        admit_mobile_pub_bytes_v1(bytes)?;
        Ok(Self {
            visual: open_mature_0x2c_geometry(bytes, environment)?,
        })
    }

    /// Opens immutable local PUB bytes using the shared Viewer environment.
    /// Platform shells should prefer this so Android/iOS do not duplicate
    /// layout-environment authority.
    pub fn open_default(bytes: &[u8]) -> Result<Self> {
        Self::open(bytes, viewer_geometry_environment_v0_1())
    }

    pub fn page_count(&self) -> usize {
        self.visual.document.pages.len()
    }

    pub fn fidelity_status(&self) -> ViewerFidelityStatus {
        self.visual.document.fidelity_status()
    }

    pub fn diagnostics(&self) -> &[ViewerDiagnostic] {
        &self.visual.document.diagnostics
    }

    pub fn search_text(&self, query: &str) -> Vec<ViewerTextMatch> {
        self.visual.document.search_text(query)
    }

    /// Returns the backend-neutral paint plan for one page.
    pub fn page_render_plan(
        &self,
        page_index: usize,
    ) -> std::result::Result<PageRenderPlanV1, RenderPlanErrorV1> {
        build_page_render_plan_v1(&self.visual, page_index)
    }

    /// Resolves encoded image bytes referenced by a render-plan resource ID.
    ///
    /// Decoding/upload into an Android or iOS texture remains a backend concern.
    pub fn image_resource_bytes(&self, resource_id: ResourceId) -> Option<&[u8]> {
        self.visual
            .images
            .iter()
            .find(|image| image.resource_id == resource_id)
            .map(|image| image.bytes.as_slice())
    }

    /// Resolves a source-neutral canonical resource key used by render plans.
    ///
    /// This keeps platform adapters from importing PUB parser/model internals
    /// just to turn the serialized render-plan resource ID back into a lookup.
    pub fn image_resource_bytes_by_key(&self, resource_key: &str) -> Result<Option<&[u8]>> {
        let canonical = resource_key
            .parse::<pub_model::CanonicalId>()
            .map_err(|error| anyhow::anyhow!("invalid render resource id: {error}"))?;
        Ok(self.image_resource_bytes(ResourceId::from_canonical(canonical)))
    }

    /// Resolves and decodes one Viewer image resource through the shared bounded
    /// image-decode contract before exposing Android-ready ARGB8888 pixels.
    pub fn admitted_image_resource_by_key(
        &self,
        resource_key: &str,
    ) -> std::result::Result<Option<MobileAdmittedImageV1>, MobileImageDecodeError> {
        let canonical = resource_key
            .parse::<pub_model::CanonicalId>()
            .map_err(|error| MobileImageDecodeError {
                code: "invalid_resource_id".to_owned(),
                detail: format!("invalid render resource id: {error}"),
            })?;
        let resource_id = ResourceId::from_canonical(canonical);
        let Some(image) = self
            .visual
            .images
            .iter()
            .find(|image| image.resource_id == resource_id)
        else {
            return Ok(None);
        };

        decode_mobile_image_v1(&image.bytes, &image.mime).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_boundary_has_stable_schema_identity() {
        assert_eq!(
            MOBILE_READER_CORE_SCHEMA_V1,
            "chaptera.mobile-reader-core.v1"
        );
    }

    #[test]
    fn mobile_manifest_does_not_declare_desktop_ui_dependencies() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["chaptera-desktop", "eframe", "egui"] {
            assert!(
                !manifest.contains(forbidden),
                "mobile core manifest must not depend on {forbidden}"
            );
        }
    }

    #[test]
    fn mobile_admission_rejects_non_cfb_before_viewer() {
        let error = admit_mobile_pub_bytes_v1(b"not a compound file")
            .expect_err("foreign bytes must fail structural admission");
        assert!(
            error
                .to_string()
                .starts_with("mobile_reader_admission.parse_failed")
        );
    }

    #[test]
    fn mobile_admission_policy_matches_ingress_limit() {
        let policy = mobile_reader_admission_policy_v1();
        assert_eq!(policy.max_file_bytes, MOBILE_READER_MAX_FILE_BYTES_V1);
        assert!(policy.max_cfb_entries > 0);
        assert!(policy.max_declared_stream_bytes > 0);
    }

    fn cfb_fixture(streams: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::{Cursor, Write};

        let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new()))
            .expect("create synthetic CFB");
        for (name, bytes) in streams {
            compound
                .create_stream(format!("/{name}"))
                .expect("create synthetic stream")
                .write_all(bytes)
                .expect("write synthetic stream");
        }
        compound.into_inner().into_inner()
    }

    #[test]
    fn mobile_admission_rejects_entry_count_and_declared_stream_bombs() {
        let two_streams = cfb_fixture(&[("A", b"x"), ("B", b"y")]);
        let entry_error = admit_mobile_pub_bytes_with_policy_v1(
            &two_streams,
            PubScanPolicyV1 {
                max_file_bytes: u64::MAX,
                max_cfb_entries: 1,
                max_declared_stream_bytes: u64::MAX,
            },
        )
        .expect_err("CFB entry-count limit must fail closed");
        assert!(
            entry_error
                .to_string()
                .contains("cfb_entry_limit:"),
            "unexpected entry-count rejection: {entry_error}"
        );

        let stream_bytes = cfb_fixture(&[("Payload", b"1234")]);
        let stream_error = admit_mobile_pub_bytes_with_policy_v1(
            &stream_bytes,
            PubScanPolicyV1 {
                max_file_bytes: u64::MAX,
                max_cfb_entries: u64::MAX,
                max_declared_stream_bytes: 3,
            },
        )
        .expect_err("declared stream byte limit must fail closed");
        assert!(
            stream_error
                .to_string()
                .contains("cfb_declared_stream_bytes_limit:"),
            "unexpected stream-byte rejection: {stream_error}"
        );
    }

    #[test]
    fn mobile_admission_rejects_oversized_input_before_cfb_walk_and_recovers_statelessly() {
        let valid_cfb = cfb_fixture(&[("Payload", b"x")]);
        let size_error = admit_mobile_pub_bytes_with_policy_v1(
            &valid_cfb,
            PubScanPolicyV1 {
                max_file_bytes: valid_cfb.len() as u64 - 1,
                max_cfb_entries: u64::MAX,
                max_declared_stream_bytes: u64::MAX,
            },
        )
        .expect_err("oversized input must fail before parse/render");
        assert!(
            size_error.to_string().contains("input_size_limit:"),
            "unexpected size rejection: {size_error}"
        );

        admit_mobile_pub_bytes_with_policy_v1(
            &valid_cfb,
            PubScanPolicyV1 {
                max_file_bytes: valid_cfb.len() as u64,
                max_cfb_entries: u64::MAX,
                max_declared_stream_bytes: u64::MAX,
            },
        )
        .expect("a later valid structural admission must not be poisoned by rejection");
    }

    #[test]
    fn public_surface_is_compile_time_read_only_facade() {
        let _open: fn(&[u8], BoundedLayoutEnvironment) -> Result<MobileReaderDocumentV1> =
            MobileReaderDocumentV1::open;
        let _open_default: fn(&[u8]) -> Result<MobileReaderDocumentV1> =
            MobileReaderDocumentV1::open_default;
        let _plan: fn(
            &MobileReaderDocumentV1,
            usize,
        ) -> std::result::Result<PageRenderPlanV1, RenderPlanErrorV1> =
            MobileReaderDocumentV1::page_render_plan;
        let _image: fn(&MobileReaderDocumentV1, ResourceId) -> Option<&[u8]> =
            MobileReaderDocumentV1::image_resource_bytes;
    }
}
