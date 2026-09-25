use crate::{
    CapabilityLevel, ExportPlan, LossItem, LossKind, LossSeverity, PlannedFeature, TargetProfile,
};
use pub_model::{CanonicalId, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Write as _;

pub const EXPORT_REPORT_SCHEMA_V0_1: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReportSource {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<Sha256Digest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReportCounts {
    pub preserved: u64,
    pub approximated: u64,
    pub flattened: u64,
    pub rasterized: u64,
    pub unsupported: u64,
    pub blocking: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReportItem {
    pub feature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<CanonicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_path: Option<String>,
    pub disposition: CapabilityLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_kind: Option<LossKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<LossSeverity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub schema_version: String,
    pub source: ExportReportSource,
    pub target: TargetProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_fence: Option<crate::ConversionFenceIdentity>,
    pub can_serialize: bool,
    pub counts: ExportReportCounts,
    pub items: Vec<ExportReportItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportReportError {
    MissingLoss {
        feature: String,
    },
    UnexpectedLoss {
        feature: String,
    },
    LossDoesNotMatchFeature {
        planned_feature: String,
        loss_feature: String,
    },
    BlockingSetMismatch,
}

impl fmt::Display for ExportReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLoss { feature } => {
                write!(
                    formatter,
                    "non-preserved feature has no loss item: {feature}"
                )
            }
            Self::UnexpectedLoss { feature } => {
                write!(
                    formatter,
                    "export plan contains unmatched loss item: {feature}"
                )
            }
            Self::LossDoesNotMatchFeature {
                planned_feature,
                loss_feature,
            } => write!(
                formatter,
                "loss item {loss_feature} does not match planned feature {planned_feature}"
            ),
            Self::BlockingSetMismatch => formatter.write_str(
                "export plan blockers are not exactly the blocking subset of loss items",
            ),
        }
    }
}

impl std::error::Error for ExportReportError {}

/// Builds one canonical report model from an ExportPlan.
///
/// Both machine-readable JSON and human-readable output are generated from this
/// value. The builder validates the public ExportPlan structure instead of
/// trusting positional coincidence silently.
pub fn build_export_report(
    plan: &ExportPlan,
    source: ExportReportSource,
) -> Result<ExportReport, ExportReportError> {
    validate_blockers(plan)?;

    let mut counts = ExportReportCounts::default();
    let mut items = Vec::with_capacity(plan.features.len());
    let mut losses = plan.losses.iter();

    for planned in &plan.features {
        let loss = match planned.disposition {
            CapabilityLevel::Preserved => None,
            _ => {
                let loss = losses
                    .next()
                    .ok_or_else(|| ExportReportError::MissingLoss {
                        feature: planned.request.feature.clone(),
                    })?;
                validate_loss_matches(planned, loss)?;
                Some(loss)
            }
        };

        count_disposition(&mut counts, planned.disposition);
        if loss.is_some_and(|item| item.severity == LossSeverity::Blocking) {
            counts.blocking += 1;
        }

        items.push(ExportReportItem {
            feature: planned.request.feature.clone(),
            origin: planned.request.origin,
            property_path: planned.request.property_path.clone(),
            disposition: planned.disposition,
            loss_kind: loss.map(|item| item.kind),
            severity: loss.map(|item| item.severity),
            reversible: loss.map(|item| item.reversible),
            code: loss.map(|item| item.code.clone()),
        });
    }

    if let Some(extra) = losses.next() {
        return Err(ExportReportError::UnexpectedLoss {
            feature: extra.feature.clone(),
        });
    }

    Ok(ExportReport {
        schema_version: EXPORT_REPORT_SCHEMA_V0_1.to_owned(),
        source,
        target: plan.target.clone(),
        conversion_fence: plan.conversion_fence.clone(),
        can_serialize: plan.can_serialize(),
        counts,
        items,
    })
}

fn validate_loss_matches(
    planned: &PlannedFeature,
    loss: &LossItem,
) -> Result<(), ExportReportError> {
    let request = &planned.request;
    if loss.feature != request.feature
        || loss.origin != request.origin
        || loss.property_path != request.property_path
    {
        return Err(ExportReportError::LossDoesNotMatchFeature {
            planned_feature: request.feature.clone(),
            loss_feature: loss.feature.clone(),
        });
    }
    Ok(())
}

fn validate_blockers(plan: &ExportPlan) -> Result<(), ExportReportError> {
    let expected = plan
        .losses
        .iter()
        .filter(|loss| loss.severity == LossSeverity::Blocking)
        .collect::<Vec<_>>();
    let actual = plan.blockers.iter().collect::<Vec<_>>();
    if expected != actual {
        return Err(ExportReportError::BlockingSetMismatch);
    }
    Ok(())
}

fn count_disposition(counts: &mut ExportReportCounts, disposition: CapabilityLevel) {
    match disposition {
        CapabilityLevel::Preserved => counts.preserved += 1,
        CapabilityLevel::Approximated => counts.approximated += 1,
        CapabilityLevel::Flattened => counts.flattened += 1,
        CapabilityLevel::Rasterized => counts.rasterized += 1,
        CapabilityLevel::Unsupported => counts.unsupported += 1,
    }
}

pub fn render_human_summary(report: &ExportReport) -> String {
    let mut output = String::new();
    writeln!(output, "source: {}", report.source.label).unwrap();
    if let Some(source_hash) = report.source.source_hash {
        writeln!(output, "source_hash: {source_hash}").unwrap();
    }
    writeln!(
        output,
        "target: {} / {} / {}",
        report.target.format, report.target.adapter_version, report.target.profile
    )
    .unwrap();
    if let Some(schema_fence) = &report.target.schema_fence {
        writeln!(output, "schema_fence: {schema_fence}").unwrap();
    }
    if let Some(fence) = &report.conversion_fence {
        writeln!(output, "conversion_fence_sha256: {}", fence.digest_sha256).unwrap();
    }
    writeln!(
        output,
        "result: {}",
        if report.can_serialize {
            if report.items.iter().any(|item| item.loss_kind.is_some()) {
                "ready_with_losses"
            } else {
                "ready"
            }
        } else {
            "blocked"
        }
    )
    .unwrap();
    writeln!(
        output,
        "counts: preserved={} approximated={} flattened={} rasterized={} unsupported={} blocking={}",
        report.counts.preserved,
        report.counts.approximated,
        report.counts.flattened,
        report.counts.rasterized,
        report.counts.unsupported,
        report.counts.blocking,
    )
    .unwrap();

    for item in report.items.iter().filter(|item| item.loss_kind.is_some()) {
        let severity = item.severity.map(severity_name).unwrap_or("unknown");
        let disposition = capability_name(item.disposition);
        write!(output, "- [{severity}] {} => {disposition}", item.feature).unwrap();
        if let Some(origin) = item.origin {
            write!(output, " @ {origin}").unwrap();
        }
        if let Some(path) = &item.property_path {
            write!(output, " ({path})").unwrap();
        }
        if let Some(code) = &item.code {
            write!(output, " [{code}]").unwrap();
        }
        if let Some(reversible) = item.reversible {
            write!(output, " reversible={reversible}").unwrap();
        }
        output.push('\n');
    }

    output
}

fn capability_name(value: CapabilityLevel) -> &'static str {
    match value {
        CapabilityLevel::Preserved => "preserved",
        CapabilityLevel::Approximated => "approximated",
        CapabilityLevel::Flattened => "flattened",
        CapabilityLevel::Rasterized => "rasterized",
        CapabilityLevel::Unsupported => "unsupported",
    }
}

fn severity_name(value: LossSeverity) -> &'static str {
    match value {
        LossSeverity::Info => "info",
        LossSeverity::Visual => "visual",
        LossSeverity::Semantic => "semantic",
        LossSeverity::Structural => "structural",
        LossSeverity::Blocking => "blocking",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SemanticFeatureRequest, TargetCapabilityManifest, plan_export};
    use std::collections::BTreeMap;

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn source_hash() -> Sha256Digest {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .expect("valid sha256")
    }

    fn target() -> TargetProfile {
        TargetProfile {
            format: "idml".into(),
            adapter_version: "idml-v0.1".into(),
            profile: "bounded-editable".into(),
            schema_fence: Some("legacy-spec-8.02/dom-7.0".into()),
        }
    }

    #[test]
    fn report_includes_preserved_and_loss_items_from_one_plan() {
        let mut features = BTreeMap::new();
        features.insert("page.geometry".into(), CapabilityLevel::Preserved);
        features.insert("effect.shadow".into(), CapabilityLevel::Flattened);
        features.insert("ole.live_object".into(), CapabilityLevel::Unsupported);
        let manifest = TargetCapabilityManifest {
            target: target(),
            features,
        };

        let plan = plan_export(
            &manifest,
            vec![
                SemanticFeatureRequest {
                    feature: "page.geometry".into(),
                    origin: Some(id(1)),
                    property_path: None,
                    require_preserved: true,
                },
                SemanticFeatureRequest {
                    feature: "effect.shadow".into(),
                    origin: Some(id(2)),
                    property_path: Some("node.effects.shadow".into()),
                    require_preserved: false,
                },
                SemanticFeatureRequest {
                    feature: "ole.live_object".into(),
                    origin: Some(id(3)),
                    property_path: None,
                    require_preserved: true,
                },
            ],
        );

        let report = build_export_report(
            &plan,
            ExportReportSource {
                label: "fixture.pub".into(),
                source_hash: Some(source_hash()),
            },
        )
        .expect("planner output should be reportable");

        assert!(!report.can_serialize);
        assert_eq!(report.counts.preserved, 1);
        assert_eq!(report.counts.flattened, 1);
        assert_eq!(report.counts.unsupported, 1);
        assert_eq!(report.counts.blocking, 1);
        assert_eq!(report.items.len(), 3);

        let machine = serde_json::to_value(&report).expect("report should serialize");
        assert_eq!(machine["counts"]["blocking"], 1);

        let human = render_human_summary(&report);
        assert!(human.contains("result: blocked"));
        assert!(human.contains(
            "counts: preserved=1 approximated=0 flattened=1 rasterized=0 unsupported=1 blocking=1"
        ));
        assert!(human.contains("[structural] effect.shadow => flattened"));
        assert!(human.contains("[blocking] ole.live_object => unsupported"));
        assert!(human.contains("export.unsupported.ole.live_object"));
    }

    #[test]
    fn report_is_deterministic_for_equivalent_request_order() {
        let mut features = BTreeMap::new();
        features.insert("page.geometry".into(), CapabilityLevel::Preserved);
        features.insert("shape.generic".into(), CapabilityLevel::Unsupported);
        let manifest = TargetCapabilityManifest {
            target: target(),
            features,
        };

        let requests = vec![
            SemanticFeatureRequest {
                feature: "shape.generic".into(),
                origin: Some(id(2)),
                property_path: None,
                require_preserved: false,
            },
            SemanticFeatureRequest {
                feature: "page.geometry".into(),
                origin: Some(id(1)),
                property_path: None,
                require_preserved: true,
            },
        ];
        let mut reversed = requests.clone();
        reversed.reverse();

        let source = ExportReportSource {
            label: "fixture.pub".into(),
            source_hash: None,
        };
        let one = build_export_report(&plan_export(&manifest, requests), source.clone())
            .expect("planner output should be reportable");
        let two = build_export_report(&plan_export(&manifest, reversed), source)
            .expect("planner output should be reportable");
        assert_eq!(one, two);
        assert_eq!(render_human_summary(&one), render_human_summary(&two));
    }

    #[test]
    fn inconsistent_public_export_plan_is_rejected() {
        let mut features = BTreeMap::new();
        features.insert("shape.generic".into(), CapabilityLevel::Unsupported);
        let manifest = TargetCapabilityManifest {
            target: target(),
            features,
        };
        let mut plan = plan_export(
            &manifest,
            vec![SemanticFeatureRequest {
                feature: "shape.generic".into(),
                origin: Some(id(2)),
                property_path: None,
                require_preserved: false,
            }],
        );
        plan.losses[0].feature = "different.feature".into();

        assert!(matches!(
            build_export_report(
                &plan,
                ExportReportSource {
                    label: "broken.pub".into(),
                    source_hash: None,
                }
            ),
            Err(ExportReportError::LossDoesNotMatchFeature { .. })
        ));
    }
}
