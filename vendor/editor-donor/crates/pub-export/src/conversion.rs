use pub_format_registry::{FormatProfileEntry, RegistryError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

pub const CONVERSION_PROFILE_SCHEMA_V0_1: &str = "pub-conversion-profile-v0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionProfile {
    pub schema_version: String,
    pub source: ConversionSourceFence,
    pub target: ConversionTargetFence,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub engine_versions: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, String>,
    pub environment: EnvironmentFence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionSourceFence {
    pub profile_id: String,
    pub adapter: String,
    pub adapter_version: String,
    pub semantic_revision: String,
    pub source_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionTargetFence {
    pub profile_id: String,
    pub format: String,
    pub adapter: String,
    pub adapter_version: String,
    pub profile_version: String,
    pub schema_fence: String,
    pub validator_profile: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentFence {
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pinned_environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionFenceIdentity {
    pub schema_version: String,
    pub digest_sha256: String,
}

#[derive(Debug)]
pub enum ConversionProfileError {
    Registry(RegistryError),
    SourceDirection,
    TargetDirection,
    MissingSchemaFence,
    TargetPlanMismatch {
        fence_format: String,
        plan_format: String,
        fence_adapter_version: String,
        plan_adapter_version: String,
    },
    Serialization(serde_json::Error),
}

impl fmt::Display for ConversionProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(error) => write!(formatter, "{error}"),
            Self::SourceDirection => {
                formatter.write_str("source profile is not a source registry entry")
            }
            Self::TargetDirection => {
                formatter.write_str("target profile is not a target registry entry")
            }
            Self::MissingSchemaFence => {
                formatter.write_str("target profile is missing a version/schema fence")
            }
            Self::TargetPlanMismatch {
                fence_format,
                plan_format,
                fence_adapter_version,
                plan_adapter_version,
            } => write!(
                formatter,
                "conversion fence target mismatch: fence={fence_format}/{fence_adapter_version}, plan={plan_format}/{plan_adapter_version}"
            ),
            Self::Serialization(error) => write!(
                formatter,
                "conversion profile serialization failed: {error}"
            ),
        }
    }
}

impl std::error::Error for ConversionProfileError {}

impl From<RegistryError> for ConversionProfileError {
    fn from(value: RegistryError) -> Self {
        Self::Registry(value)
    }
}

impl ConversionProfile {
    pub fn from_registry(
        source_profile_id: &str,
        target_profile_id: &str,
        semantic_revision: impl Into<String>,
        source_sha256: impl Into<String>,
        environment: EnvironmentFence,
    ) -> Result<Self, ConversionProfileError> {
        let source = pub_format_registry::resolve(source_profile_id)?;
        let target = pub_format_registry::resolve(target_profile_id)?;
        Self::from_entries(
            source,
            target,
            semantic_revision.into(),
            source_sha256.into(),
            environment,
        )
    }

    fn from_entries(
        source: FormatProfileEntry,
        target: FormatProfileEntry,
        semantic_revision: String,
        source_sha256: String,
        environment: EnvironmentFence,
    ) -> Result<Self, ConversionProfileError> {
        if source.direction != pub_format_registry::FormatDirection::Source {
            return Err(ConversionProfileError::SourceDirection);
        }
        if target.direction != pub_format_registry::FormatDirection::Target {
            return Err(ConversionProfileError::TargetDirection);
        }
        if target.specification.version_fence.is_empty() {
            return Err(ConversionProfileError::MissingSchemaFence);
        }

        Ok(Self {
            schema_version: CONVERSION_PROFILE_SCHEMA_V0_1.to_owned(),
            source: ConversionSourceFence {
                profile_id: source.profile_id,
                adapter: source.adapter.crate_name,
                adapter_version: source.adapter.implementation_version,
                semantic_revision,
                source_sha256,
            },
            target: ConversionTargetFence {
                profile_id: target.profile_id,
                format: target.format,
                adapter: target.adapter.crate_name,
                adapter_version: target.adapter.implementation_version,
                profile_version: target.profile_version,
                schema_fence: target.specification.version_fence,
                validator_profile: target.validator.profile_id,
                validator_version: target.validator.version,
            },
            engine_versions: BTreeMap::new(),
            resources: BTreeMap::new(),
            environment,
        })
    }

    pub fn validate_target(
        &self,
        target: &crate::TargetProfile,
    ) -> Result<(), ConversionProfileError> {
        if self.target.format != target.format
            || self.target.adapter_version != target.adapter_version
        {
            return Err(ConversionProfileError::TargetPlanMismatch {
                fence_format: self.target.format.clone(),
                plan_format: target.format.clone(),
                fence_adapter_version: self.target.adapter_version.clone(),
                plan_adapter_version: target.adapter_version.clone(),
            });
        }
        Ok(())
    }

    pub fn identity(&self) -> Result<ConversionFenceIdentity, ConversionProfileError> {
        let bytes = serde_json::to_vec(self).map_err(ConversionProfileError::Serialization)?;
        let digest = Sha256::digest(bytes);
        let digest_sha256 = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(ConversionFenceIdentity {
            schema_version: CONVERSION_PROFILE_SCHEMA_V0_1.to_owned(),
            digest_sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> EnvironmentFence {
        EnvironmentFence {
            policy_version: "conversion-policy-v0.1".into(),
            renderer_version: None,
            pinned_environment: BTreeMap::from([
                ("locale".into(), "C".into()),
                ("timezone".into(), "UTC".into()),
            ]),
        }
    }

    fn profile(target: &str) -> ConversionProfile {
        let mut value = ConversionProfile::from_registry(
            "pub-mature-0x2c-v0.1",
            target,
            "semantic-revision-7",
            "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf",
            environment(),
        )
        .expect("known registry profiles should compose");
        value
            .engine_versions
            .insert("pub-export".into(), "export-plan-v0.1".into());
        value
            .resources
            .insert("font.fallback".into(), "sha256:abc123".into());
        value
    }

    #[test]
    fn identical_fence_has_identical_digest() {
        let one = profile("idml-bounded-v0.1").identity().unwrap();
        let two = profile("idml-bounded-v0.1").identity().unwrap();
        assert_eq!(one, two);
    }

    #[test]
    fn material_resource_change_changes_digest() {
        let one = profile("idml-bounded-v0.1");
        let mut two = one.clone();
        two.resources
            .insert("font.fallback".into(), "sha256:def456".into());
        assert_ne!(one.identity().unwrap(), two.identity().unwrap());
    }

    #[test]
    fn target_profile_change_changes_digest() {
        let one = profile("idml-bounded-v0.1").identity().unwrap();
        let two = profile("odg-bounded-v0.1").identity().unwrap();
        assert_ne!(one, two);
    }

    #[test]
    fn mismatched_plan_target_fails_closed() {
        let profile = profile("idml-bounded-v0.1");
        let target = crate::TargetProfile {
            format: "odg".into(),
            adapter_version: "odg-v0.1".into(),
            profile: "bounded".into(),
            schema_fence: Some("odf-1.4".into()),
        };
        assert!(matches!(
            profile.validate_target(&target),
            Err(ConversionProfileError::TargetPlanMismatch { .. })
        ));
    }

    #[test]
    fn unknown_profile_fails_closed() {
        let error = ConversionProfile::from_registry(
            "pub-mature-0x2c-v0.1",
            "missing-target",
            "revision",
            "hash",
            environment(),
        )
        .expect_err("unknown registry profile must fail closed");
        assert!(matches!(error, ConversionProfileError::Registry(_)));
    }
}
