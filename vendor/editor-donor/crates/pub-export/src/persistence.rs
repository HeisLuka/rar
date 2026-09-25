//! Оценка совместимости authoring semantics с persistence target.
//!
//! Этот модуль намеренно разделяет два независимых вопроса:
//! 1. способен ли сам формат/профиль представить семантику;
//! 2. умеет ли текущий writer безопасно записать её.
//!
//! Отсутствие доказательства о формате не означает несовместимость. Оно остаётся
//! состоянием `NotEvaluated`. Аналогично отсутствие writer-proof блокирует
//! native save, но не превращает feature в format-incompatible.

use pub_model::CanonicalId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PERSISTENCE_COMPATIBILITY_SCHEMA_V0_1: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PersistenceTargetProfile {
    pub format: String,
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_fence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatRepresentability {
    Lossless,
    WithLoss,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterCapability {
    Writable,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PersistenceRequirement {
    pub feature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<CanonicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedFormatCapability {
    pub requirement: PersistenceRequirement,
    pub capability: FormatRepresentability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatCompatibilityManifest {
    pub target: PersistenceTargetProfile,
    /// Стабильный ключ semantic feature -> доказанная представимость по умолчанию.
    ///
    /// Отсутствующий ключ намеренно означает "not evaluated", а не неявную несовместимость.
    pub features: BTreeMap<String, FormatRepresentability>,
    /// Более узкие доказательства для конкретного semantic origin/property.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoped: Vec<ScopedFormatCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedWriterCapability {
    pub requirement: PersistenceRequirement,
    pub capability: WriterCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterCapabilityManifest {
    pub target: PersistenceTargetProfile,
    pub writer_version: String,
    /// Стабильный ключ semantic feature -> доказанная возможность writer по умолчанию.
    ///
    /// Отсутствующий ключ означает, что для этой feature ещё нет writer-proof.
    pub features: BTreeMap<String, WriterCapability>,
    /// Source/operation-specific writer probe для конкретного semantic requirement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoped: Vec<ScopedWriterCapability>,
}

/// Небольшая граница для будущих editor/product operations.
///
/// Конкретный EditOperation может реализовать этот trait без зависимости этого crate
/// от editor crate. Evaluator видит только semantic requirements.
pub trait PersistenceRequirements {
    fn persistence_requirements(&self) -> Vec<PersistenceRequirement>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceCompatibilityState {
    Writable,
    WriterBlocked,
    RequiresLoss,
    FormatIncompatible,
    NotEvaluated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistenceCompatibilityItem {
    pub requirement: PersistenceRequirement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<FormatRepresentability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writer: Option<WriterCapability>,
    pub state: PersistenceCompatibilityState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistenceCompatibilityAssessment {
    pub schema_version: String,
    pub target: PersistenceTargetProfile,
    pub writer_version: String,
    pub state: PersistenceCompatibilityState,
    pub items: Vec<PersistenceCompatibilityItem>,
}

impl PersistenceCompatibilityAssessment {
    /// Это только compatibility/writer часть допуска к native save.
    ///
    /// Перед показом или выполнением native save вызывающая сторона всё равно обязана
    /// проверить source-preservation и validation/round-trip gates.
    pub fn can_write_losslessly(&self) -> bool {
        self.state == PersistenceCompatibilityState::Writable
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceCompatibilityError {
    TargetProfileMismatch {
        format_target: Box<PersistenceTargetProfile>,
        writer_target: Box<PersistenceTargetProfile>,
    },
    DuplicateScopedFormatCapability {
        requirement: Box<PersistenceRequirement>,
    },
    DuplicateScopedWriterCapability {
        requirement: Box<PersistenceRequirement>,
    },
}

impl std::fmt::Display for PersistenceCompatibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetProfileMismatch {
                format_target,
                writer_target,
            } => write!(
                f,
                "несовпадение target profile у format/writer: format={format_target:?}, writer={writer_target:?}"
            ),
            Self::DuplicateScopedFormatCapability { requirement } => write!(
                f,
                "повторный scoped format capability для requirement {requirement:?}"
            ),
            Self::DuplicateScopedWriterCapability { requirement } => write!(
                f,
                "повторный scoped writer capability для requirement {requirement:?}"
            ),
        }
    }
}

impl std::error::Error for PersistenceCompatibilityError {}

pub fn assess_persistence_compatibility(
    format: &FormatCompatibilityManifest,
    writer: &WriterCapabilityManifest,
    mut requirements: Vec<PersistenceRequirement>,
) -> Result<PersistenceCompatibilityAssessment, PersistenceCompatibilityError> {
    if format.target != writer.target {
        return Err(PersistenceCompatibilityError::TargetProfileMismatch {
            format_target: Box::new(format.target.clone()),
            writer_target: Box::new(writer.target.clone()),
        });
    }

    requirements.sort();
    requirements.dedup();

    let mut items = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        let format_level = scoped_format_capability(format, &requirement)?
            .or_else(|| format.features.get(&requirement.feature).copied());
        let writer_level = scoped_writer_capability(writer, &requirement)?
            .or_else(|| writer.features.get(&requirement.feature).copied());

        let state = match format_level {
            None => PersistenceCompatibilityState::NotEvaluated,
            Some(FormatRepresentability::Incompatible) => {
                PersistenceCompatibilityState::FormatIncompatible
            }
            Some(FormatRepresentability::WithLoss) => PersistenceCompatibilityState::RequiresLoss,
            Some(FormatRepresentability::Lossless) => match writer_level {
                Some(WriterCapability::Writable) => PersistenceCompatibilityState::Writable,
                Some(WriterCapability::Blocked) | None => {
                    PersistenceCompatibilityState::WriterBlocked
                }
            },
        };

        items.push(PersistenceCompatibilityItem {
            requirement,
            format: format_level,
            writer: writer_level,
            state,
        });
    }

    let state = aggregate_state(items.iter().map(|item| item.state));

    Ok(PersistenceCompatibilityAssessment {
        schema_version: PERSISTENCE_COMPATIBILITY_SCHEMA_V0_1.to_owned(),
        target: format.target.clone(),
        writer_version: writer.writer_version.clone(),
        state,
        items,
    })
}

fn scoped_format_capability(
    manifest: &FormatCompatibilityManifest,
    requirement: &PersistenceRequirement,
) -> Result<Option<FormatRepresentability>, PersistenceCompatibilityError> {
    let mut matches = manifest
        .scoped
        .iter()
        .filter(|entry| &entry.requirement == requirement);
    let capability = matches.next().map(|entry| entry.capability);
    if matches.next().is_some() {
        return Err(
            PersistenceCompatibilityError::DuplicateScopedFormatCapability {
                requirement: Box::new(requirement.clone()),
            },
        );
    }
    Ok(capability)
}

fn scoped_writer_capability(
    manifest: &WriterCapabilityManifest,
    requirement: &PersistenceRequirement,
) -> Result<Option<WriterCapability>, PersistenceCompatibilityError> {
    let mut matches = manifest
        .scoped
        .iter()
        .filter(|entry| &entry.requirement == requirement);
    let capability = matches.next().map(|entry| entry.capability);
    if matches.next().is_some() {
        return Err(
            PersistenceCompatibilityError::DuplicateScopedWriterCapability {
                requirement: Box::new(requirement.clone()),
            },
        );
    }
    Ok(capability)
}

fn aggregate_state(
    states: impl IntoIterator<Item = PersistenceCompatibilityState>,
) -> PersistenceCompatibilityState {
    let mut aggregate = PersistenceCompatibilityState::Writable;
    for state in states {
        aggregate = worse_state(aggregate, state);
    }
    aggregate
}

fn worse_state(
    left: PersistenceCompatibilityState,
    right: PersistenceCompatibilityState,
) -> PersistenceCompatibilityState {
    if state_rank(right) > state_rank(left) {
        right
    } else {
        left
    }
}

fn state_rank(state: PersistenceCompatibilityState) -> u8 {
    match state {
        PersistenceCompatibilityState::Writable => 0,
        PersistenceCompatibilityState::WriterBlocked => 1,
        PersistenceCompatibilityState::NotEvaluated => 2,
        PersistenceCompatibilityState::RequiresLoss => 3,
        PersistenceCompatibilityState::FormatIncompatible => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn target() -> PersistenceTargetProfile {
        PersistenceTargetProfile {
            format: "pub".into(),
            profile: "publisher-2010".into(),
            schema_fence: Some("pub-family-0x2c".into()),
        }
    }

    fn request(feature: &str, byte: u8) -> PersistenceRequirement {
        PersistenceRequirement {
            feature: feature.into(),
            origin: Some(id(byte)),
            property_path: None,
        }
    }

    fn manifests(
        format_level: Option<FormatRepresentability>,
        writer_level: Option<WriterCapability>,
    ) -> (FormatCompatibilityManifest, WriterCapabilityManifest) {
        let mut format_features = BTreeMap::new();
        if let Some(level) = format_level {
            format_features.insert("story.text".into(), level);
        }

        let mut writer_features = BTreeMap::new();
        if let Some(level) = writer_level {
            writer_features.insert("story.text".into(), level);
        }

        (
            FormatCompatibilityManifest {
                target: target(),
                features: format_features,
                scoped: Vec::new(),
            },
            WriterCapabilityManifest {
                target: target(),
                writer_version: "pub-writer-v0.1".into(),
                features: writer_features,
                scoped: Vec::new(),
            },
        )
    }

    #[test]
    fn lossless_and_writable_is_writable() {
        let (format, writer) = manifests(
            Some(FormatRepresentability::Lossless),
            Some(WriterCapability::Writable),
        );

        let assessment =
            assess_persistence_compatibility(&format, &writer, vec![request("story.text", 1)])
                .expect("matching profiles");

        assert_eq!(assessment.state, PersistenceCompatibilityState::Writable);
        assert!(assessment.can_write_losslessly());
    }

    #[test]
    fn proven_format_but_missing_writer_is_writer_blocked_not_incompatible() {
        let (format, writer) = manifests(Some(FormatRepresentability::Lossless), None);

        let assessment =
            assess_persistence_compatibility(&format, &writer, vec![request("story.text", 1)])
                .expect("matching profiles");

        assert_eq!(
            assessment.state,
            PersistenceCompatibilityState::WriterBlocked
        );
        assert_eq!(assessment.items[0].writer, None);
        assert!(!assessment.can_write_losslessly());
    }

    #[test]
    fn explicit_lossy_format_representation_requires_loss() {
        let (format, writer) = manifests(
            Some(FormatRepresentability::WithLoss),
            Some(WriterCapability::Writable),
        );

        let assessment =
            assess_persistence_compatibility(&format, &writer, vec![request("story.text", 1)])
                .expect("matching profiles");

        assert_eq!(
            assessment.state,
            PersistenceCompatibilityState::RequiresLoss
        );
    }

    #[test]
    fn explicit_format_incompatibility_is_distinct_from_writer_state() {
        let (format, writer) = manifests(
            Some(FormatRepresentability::Incompatible),
            Some(WriterCapability::Writable),
        );

        let assessment =
            assess_persistence_compatibility(&format, &writer, vec![request("story.text", 1)])
                .expect("matching profiles");

        assert_eq!(
            assessment.state,
            PersistenceCompatibilityState::FormatIncompatible
        );
    }

    #[test]
    fn missing_format_evidence_is_not_evaluated_not_unsupported() {
        let (format, writer) = manifests(None, Some(WriterCapability::Writable));

        let assessment =
            assess_persistence_compatibility(&format, &writer, vec![request("story.text", 1)])
                .expect("matching profiles");

        assert_eq!(
            assessment.state,
            PersistenceCompatibilityState::NotEvaluated
        );
        assert_eq!(assessment.items[0].format, None);
    }

    #[test]
    fn aggregation_is_deterministic_and_uses_strongest_blocker() {
        let mut format_features = BTreeMap::new();
        format_features.insert("story.text".into(), FormatRepresentability::Lossless);
        format_features.insert(
            "component.modern".into(),
            FormatRepresentability::Incompatible,
        );

        let mut writer_features = BTreeMap::new();
        writer_features.insert("story.text".into(), WriterCapability::Writable);

        let format = FormatCompatibilityManifest {
            target: target(),
            features: format_features,
            scoped: Vec::new(),
        };
        let writer = WriterCapabilityManifest {
            target: target(),
            writer_version: "pub-writer-v0.1".into(),
            features: writer_features,
            scoped: Vec::new(),
        };

        let one = vec![request("story.text", 1), request("component.modern", 2)];
        let mut two = one.clone();
        two.reverse();

        let a = assess_persistence_compatibility(&format, &writer, one).unwrap();
        let b = assess_persistence_compatibility(&format, &writer, two).unwrap();

        assert_eq!(a, b);
        assert_eq!(a.state, PersistenceCompatibilityState::FormatIncompatible);
    }

    #[test]
    fn scoped_writer_capability_can_prove_one_origin_without_promoting_the_feature() {
        let (format, mut writer) = manifests(Some(FormatRepresentability::Lossless), None);
        let first = request("story.text", 1);
        let second = request("story.text", 2);
        writer.scoped.push(ScopedWriterCapability {
            requirement: first.clone(),
            capability: WriterCapability::Writable,
        });

        let first_assessment =
            assess_persistence_compatibility(&format, &writer, vec![first.clone()]).unwrap();
        assert_eq!(
            first_assessment.state,
            PersistenceCompatibilityState::Writable
        );

        let both = assess_persistence_compatibility(&format, &writer, vec![second, first]).unwrap();
        assert_eq!(both.state, PersistenceCompatibilityState::WriterBlocked);
    }

    #[test]
    fn mismatched_profiles_are_rejected() {
        let (format, mut writer) = manifests(
            Some(FormatRepresentability::Lossless),
            Some(WriterCapability::Writable),
        );
        writer.target.profile = "publisher-2007".into();

        assert!(matches!(
            assess_persistence_compatibility(&format, &writer, vec![request("story.text", 1)]),
            Err(PersistenceCompatibilityError::TargetProfileMismatch { .. })
        ));
    }
}
