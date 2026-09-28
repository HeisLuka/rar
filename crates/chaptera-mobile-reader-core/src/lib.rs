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
    let result = inspect_pub_bytes_v1(bytes, mobile_reader_admission_policy_v1(), false);
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
