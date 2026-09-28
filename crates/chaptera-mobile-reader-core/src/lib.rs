//! Platform-neutral Chaptera Reader boundary intended for mobile shells.
//!
//! This crate deliberately owns no Android, JNI, Kotlin, egui, filesystem, or
//! source-format semantics. A mobile shell supplies local PUB bytes plus a
//! bounded layout environment and receives Viewer/render-plan facts produced by
//! the same Reader pipeline used by the desktop product.

use anyhow::Result;

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
