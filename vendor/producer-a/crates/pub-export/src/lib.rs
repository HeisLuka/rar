//! Target-neutral export planning and loss classification.
//!
//! This crate sits between authoring semantics and concrete target adapters.
//! It deliberately does not serialize IDML/ODG/etc. and does not depend on
//! PUB source parser crates.

mod conversion;
mod persistence;
mod report;

pub use conversion::{
    CONVERSION_PROFILE_SCHEMA_V0_1, ConversionFenceIdentity, ConversionProfile,
    ConversionProfileError, ConversionSourceFence, ConversionTargetFence, EnvironmentFence,
};
pub use persistence::{
    FormatCompatibilityManifest, FormatRepresentability, PERSISTENCE_COMPATIBILITY_SCHEMA_V0_1,
    PersistenceCompatibilityAssessment, PersistenceCompatibilityError,
    PersistenceCompatibilityItem, PersistenceCompatibilityState, PersistenceRequirement,
    PersistenceRequirements, PersistenceTargetProfile, ScopedFormatCapability,
    ScopedWriterCapability, WriterCapability, WriterCapabilityManifest,
    assess_persistence_compatibility,
};

pub use report::{
    EXPORT_REPORT_SCHEMA_V0_1, ExportReport, ExportReportCounts, ExportReportError,
    ExportReportItem, ExportReportSource, build_export_report, render_human_summary,
};

use pub_model::CanonicalId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const EXPORT_PLAN_SCHEMA_V0_1: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TargetProfile {
    pub format: String,
    pub adapter_version: String,
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_fence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLevel {
    Preserved,
    Approximated,
    Flattened,
    Rasterized,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCapabilityManifest {
    pub target: TargetProfile,
    /// Semantic feature key -> strongest disposition the adapter declares.
    pub features: BTreeMap<String, CapabilityLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SemanticFeatureRequest {
    /// Stable semantic feature vocabulary, e.g. "page.geometry" or
    /// "story.linked_frames".
    pub feature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<CanonicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_path: Option<String>,
    /// If true, any non-preserved outcome becomes blocking for this export.
    pub require_preserved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LossKind {
    Approximated,
    Flattened,
    Rasterized,
    TextReflowed,
    FontSubstituted,
    Unsupported,
    SourceExtensionLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LossSeverity {
    Info,
    Visual,
    Semantic,
    Structural,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossItem {
    pub feature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<CanonicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_path: Option<String>,
    pub kind: LossKind,
    pub severity: LossSeverity,
    pub target: TargetProfile,
    pub reversible: bool,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFeature {
    pub request: SemanticFeatureRequest,
    pub disposition: CapabilityLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportPlan {
    pub schema_version: String,
    pub target: TargetProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_fence: Option<ConversionFenceIdentity>,
    pub features: Vec<PlannedFeature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<LossItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LossItem>,
}

impl ExportPlan {
    pub fn can_serialize(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// Plans an export without serializing the target.
///
/// Input order is explicitly non-semantic. Requests are normalized so the same
/// semantic requirements + target manifest produce the same plan.
pub fn plan_export(
    manifest: &TargetCapabilityManifest,
    mut requests: Vec<SemanticFeatureRequest>,
) -> ExportPlan {
    requests.sort();

    let mut features = Vec::with_capacity(requests.len());
    let mut losses = Vec::new();
    let mut blockers = Vec::new();

    for request in requests {
        let disposition = manifest
            .features
            .get(&request.feature)
            .copied()
            .unwrap_or(CapabilityLevel::Unsupported);

        let planned = PlannedFeature {
            request: request.clone(),
            disposition,
        };

        if disposition != CapabilityLevel::Preserved {
            let loss = loss_for(&manifest.target, &request, disposition);
            if loss.severity == LossSeverity::Blocking {
                blockers.push(loss.clone());
            }
            losses.push(loss);
        }

        features.push(planned);
    }

    ExportPlan {
        schema_version: EXPORT_PLAN_SCHEMA_V0_1.to_owned(),
        target: manifest.target.clone(),
        conversion_fence: None,
        features,
        losses,
        blockers,
    }
}

pub fn plan_export_fenced(
    manifest: &TargetCapabilityManifest,
    requests: Vec<SemanticFeatureRequest>,
    profile: &ConversionProfile,
) -> Result<ExportPlan, ConversionProfileError> {
    profile.validate_target(&manifest.target)?;
    let mut plan = plan_export(manifest, requests);
    plan.conversion_fence = Some(profile.identity()?);
    Ok(plan)
}

fn loss_for(
    target: &TargetProfile,
    request: &SemanticFeatureRequest,
    disposition: CapabilityLevel,
) -> LossItem {
    let (kind, default_severity, reversible, suffix) = match disposition {
        CapabilityLevel::Preserved => unreachable!("preserved feature has no loss"),
        CapabilityLevel::Approximated => (
            LossKind::Approximated,
            LossSeverity::Semantic,
            true,
            "approximated",
        ),
        CapabilityLevel::Flattened => (
            LossKind::Flattened,
            LossSeverity::Structural,
            false,
            "flattened",
        ),
        CapabilityLevel::Rasterized => (
            LossKind::Rasterized,
            LossSeverity::Structural,
            false,
            "rasterized",
        ),
        CapabilityLevel::Unsupported => (
            LossKind::Unsupported,
            LossSeverity::Semantic,
            false,
            "unsupported",
        ),
    };

    let severity = if request.require_preserved {
        LossSeverity::Blocking
    } else {
        default_severity
    };

    LossItem {
        feature: request.feature.clone(),
        origin: request.origin,
        property_path: request.property_path.clone(),
        kind,
        severity,
        target: target.clone(),
        reversible,
        code: format!("export.{suffix}.{}", request.feature),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn target() -> TargetProfile {
        TargetProfile {
            format: "idml".into(),
            adapter_version: "idml-v0.1".into(),
            profile: "bounded-editable".into(),
            schema_fence: Some("legacy-spec-8.02".into()),
        }
    }

    #[test]
    fn plan_is_deterministic_independent_of_request_order() {
        let mut features = BTreeMap::new();
        features.insert("page.geometry".into(), CapabilityLevel::Preserved);
        features.insert("story.linked_frames".into(), CapabilityLevel::Preserved);
        let manifest = TargetCapabilityManifest {
            target: target(),
            features,
        };

        let a = vec![
            SemanticFeatureRequest {
                feature: "story.linked_frames".into(),
                origin: Some(id(2)),
                property_path: None,
                require_preserved: true,
            },
            SemanticFeatureRequest {
                feature: "page.geometry".into(),
                origin: Some(id(1)),
                property_path: None,
                require_preserved: true,
            },
        ];

        let mut b = a.clone();
        b.reverse();

        assert_eq!(plan_export(&manifest, a), plan_export(&manifest, b));
    }

    #[test]
    fn missing_required_capability_is_blocking() {
        let manifest = TargetCapabilityManifest {
            target: target(),
            features: BTreeMap::new(),
        };

        let plan = plan_export(
            &manifest,
            vec![SemanticFeatureRequest {
                feature: "table.structure".into(),
                origin: Some(id(3)),
                property_path: Some("node.table".into()),
                require_preserved: true,
            }],
        );

        assert!(!plan.can_serialize());
        assert_eq!(plan.losses.len(), 1);
        assert_eq!(plan.blockers.len(), 1);
        assert_eq!(plan.blockers[0].severity, LossSeverity::Blocking);
        assert_eq!(plan.blockers[0].kind, LossKind::Unsupported);
    }

    #[test]
    fn non_required_downgrade_is_reported_but_not_blocking() {
        let mut features = BTreeMap::new();
        features.insert("effect.shadow".into(), CapabilityLevel::Flattened);
        let manifest = TargetCapabilityManifest {
            target: target(),
            features,
        };

        let plan = plan_export(
            &manifest,
            vec![SemanticFeatureRequest {
                feature: "effect.shadow".into(),
                origin: Some(id(4)),
                property_path: None,
                require_preserved: false,
            }],
        );

        assert!(plan.can_serialize());
        assert_eq!(plan.losses[0].kind, LossKind::Flattened);
        assert_eq!(plan.losses[0].severity, LossSeverity::Structural);
    }

    #[test]
    fn json_is_stable_after_normalization() {
        let mut features = BTreeMap::new();
        features.insert("page.geometry".into(), CapabilityLevel::Preserved);
        let manifest = TargetCapabilityManifest {
            target: target(),
            features,
        };
        let requests = vec![SemanticFeatureRequest {
            feature: "page.geometry".into(),
            origin: Some(id(1)),
            property_path: None,
            require_preserved: true,
        }];

        let one = serde_json::to_string(&plan_export(&manifest, requests.clone()))
            .expect("plan should serialize");
        let two = serde_json::to_string(&plan_export(&manifest, requests))
            .expect("plan should serialize");
        assert_eq!(one, two);
    }
}
