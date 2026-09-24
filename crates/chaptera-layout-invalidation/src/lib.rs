use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type FingerprintV1 = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DependencyNodeV1 {
    Node(String),
    Story(String),
    Page(String),
    Style(String),
    Resource(String),
    LayoutEnvironment(String),
    PageOrder,
    GlobalDefaults,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DependencyKindV1 {
    Semantic,
    Geometry,
    Typography,
    WrapConstraint,
    FlowTopology,
    Resource,
    Order,
    Environment,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtifactKeyV1 {
    LayoutProjection { owner: String },
    PreparedTypography { story_id: String, unit_id: String },
    LineRegion { frame_id: String },
    StoryFlow { story_id: String, frame_id: String },
    SceneShard { page_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DependencyEdgeV1 {
    pub source: DependencyNodeV1,
    pub kind: DependencyKindV1,
    pub observed_fingerprint: FingerprintV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactReceiptV1 {
    pub artifact: ArtifactKeyV1,
    pub observed_revision: String,
    pub dependency_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub stage_version: String,
    pub dependencies: Vec<DependencyEdgeV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationChangeV1 {
    Changed,
    Unchanged,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedUpstreamV1 {
    pub artifact: ArtifactKeyV1,
    pub output_fingerprint: FingerprintV1,
}

#[derive(Clone, Debug)]
pub struct ComputedArtifactV1 {
    pub receipt: ArtifactReceiptV1,
    pub expected_upstream: Vec<ExpectedUpstreamV1>,
    pub output_comparable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishErrorV1 {
    StaleUpstream { artifact: ArtifactKeyV1 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationResultV1 {
    pub change: PublicationChangeV1,
}

#[derive(Default)]
pub struct InvalidationGraphV1 {
    artifacts: HashMap<ArtifactKeyV1, ArtifactReceiptV1>,
    reverse: BTreeMap<DependencyNodeV1, BTreeSet<ArtifactKeyV1>>,
}

impl InvalidationGraphV1 {
    pub fn receipt(&self, key: &ArtifactKeyV1) -> Option<&ArtifactReceiptV1> {
        self.artifacts.get(key)
    }

    pub fn consumers_of(&self, source: &DependencyNodeV1) -> BTreeSet<ArtifactKeyV1> {
        self.reverse.get(source).cloned().unwrap_or_default()
    }

    pub fn dependency_fingerprint_matches(
        &self,
        key: &ArtifactKeyV1,
        fingerprint: FingerprintV1,
    ) -> bool {
        self.receipt(key)
            .is_some_and(|receipt| receipt.dependency_fingerprint == fingerprint)
    }

    pub fn publish(
        &mut self,
        computed: ComputedArtifactV1,
    ) -> Result<PublicationResultV1, PublishErrorV1> {
        for expected in &computed.expected_upstream {
            let actual = self
                .receipt(&expected.artifact)
                .map(|receipt| receipt.output_fingerprint);
            if actual != Some(expected.output_fingerprint) {
                return Err(PublishErrorV1::StaleUpstream {
                    artifact: expected.artifact.clone(),
                });
            }
        }

        let key = computed.receipt.artifact.clone();
        let prior = self.artifacts.get(&key).cloned();

        if let Some(prior) = &prior {
            for dep in &prior.dependencies {
                if let Some(consumers) = self.reverse.get_mut(&dep.source) {
                    consumers.remove(&key);
                    if consumers.is_empty() {
                        self.reverse.remove(&dep.source);
                    }
                }
            }
        }

        let mut receipt = computed.receipt;
        receipt.dependencies.sort();
        receipt.dependencies.dedup();

        for dep in &receipt.dependencies {
            self.reverse
                .entry(dep.source.clone())
                .or_default()
                .insert(key.clone());
        }

        let change = match prior {
            None => PublicationChangeV1::Changed,
            Some(old) if !computed.output_comparable => PublicationChangeV1::Unknown,
            Some(old) if old.stage_version != receipt.stage_version => PublicationChangeV1::Unknown,
            Some(old) if old.output_fingerprint == receipt.output_fingerprint => {
                PublicationChangeV1::Unchanged
            }
            Some(_) => PublicationChangeV1::Changed,
        };

        self.artifacts.insert(key, receipt);
        Ok(PublicationResultV1 { change })
    }
}

pub fn fingerprint_v1(domain: &str, parts: &[&[u8]]) -> FingerprintV1 {
    let mut hasher = Sha256::new();
    write_len_prefixed(&mut hasher, domain.as_bytes());
    for part in parts {
        write_len_prefixed(&mut hasher, part);
    }
    hasher.finalize().into()
}

fn write_len_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).expect("fingerprint input length fits u64");
    hasher.update(len.to_be_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(domain: &str, value: &str) -> FingerprintV1 {
        fingerprint_v1(domain, &[value.as_bytes()])
    }

    fn edge(source: DependencyNodeV1, kind: DependencyKindV1, value: &str) -> DependencyEdgeV1 {
        DependencyEdgeV1 {
            source,
            kind,
            observed_fingerprint: fp("dep-v1", value),
        }
    }

    fn receipt(
        artifact: ArtifactKeyV1,
        revision: &str,
        input: &str,
        output: &str,
        deps: Vec<DependencyEdgeV1>,
    ) -> ArtifactReceiptV1 {
        ArtifactReceiptV1 {
            artifact,
            observed_revision: revision.to_owned(),
            dependency_fingerprint: fp("input-v1", input),
            output_fingerprint: fp("output-v1", output),
            stage_version: "stage-v1".to_owned(),
            dependencies: deps,
        }
    }

    #[test]
    fn revision_change_does_not_force_dependency_fingerprint_miss() {
        let key = ArtifactKeyV1::PreparedTypography {
            story_id: "s1".into(),
            unit_id: "u1".into(),
        };
        let mut graph = InvalidationGraphV1::default();
        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(key.clone(), "r1", "same", "same", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        assert!(graph.dependency_fingerprint_matches(&key, fp("input-v1", "same")));

        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(key.clone(), "r2", "same", "same", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        assert!(graph.dependency_fingerprint_matches(&key, fp("input-v1", "same")));
        assert_eq!(graph.receipt(&key).unwrap().observed_revision, "r2");
    }

    #[test]
    fn recompute_with_same_output_stops_as_unchanged() {
        let key = ArtifactKeyV1::LayoutProjection { owner: "n1".into() };
        let mut graph = InvalidationGraphV1::default();
        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(key.clone(), "r1", "input-a", "public", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        let result = graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(key, "r2", "input-b", "public", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        assert_eq!(result.change, PublicationChangeV1::Unchanged);
    }

    #[test]
    fn dependency_replacement_removes_obsolete_reverse_edge() {
        let key = ArtifactKeyV1::SceneShard { page_id: "p1".into() };
        let a = DependencyNodeV1::Resource("a".into());
        let b = DependencyNodeV1::Resource("b".into());
        let mut graph = InvalidationGraphV1::default();

        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(
                    key.clone(),
                    "r1",
                    "a",
                    "scene-a",
                    vec![edge(a.clone(), DependencyKindV1::Resource, "a")],
                ),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(
                    key.clone(),
                    "r2",
                    "b",
                    "scene-b",
                    vec![edge(b.clone(), DependencyKindV1::Resource, "b")],
                ),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        assert!(!graph.consumers_of(&a).contains(&key));
        assert!(graph.consumers_of(&b).contains(&key));
    }

    #[test]
    fn unknown_is_conservative_when_output_not_comparable() {
        let key = ArtifactKeyV1::StoryFlow {
            story_id: "s1".into(),
            frame_id: "f1".into(),
        };
        let mut graph = InvalidationGraphV1::default();
        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(key.clone(), "r1", "a", "x", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        let result = graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(key, "r2", "b", "x", vec![]),
                expected_upstream: vec![],
                output_comparable: false,
            })
            .unwrap();

        assert_eq!(result.change, PublicationChangeV1::Unknown);
    }

    #[test]
    fn stale_worker_result_is_rejected() {
        let upstream = ArtifactKeyV1::LineRegion {
            frame_id: "f1".into(),
        };
        let downstream = ArtifactKeyV1::StoryFlow {
            story_id: "s1".into(),
            frame_id: "f1".into(),
        };
        let mut graph = InvalidationGraphV1::default();

        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(upstream.clone(), "r1", "a", "region-a", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        let expected_old = graph.receipt(&upstream).unwrap().output_fingerprint;

        graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(upstream.clone(), "r2", "b", "region-b", vec![]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .unwrap();

        let err = graph
            .publish(ComputedArtifactV1 {
                receipt: receipt(downstream, "r2", "flow", "flow", vec![]),
                expected_upstream: vec![ExpectedUpstreamV1 {
                    artifact: upstream.clone(),
                    output_fingerprint: expected_old,
                }],
                output_comparable: true,
            })
            .unwrap_err();

        assert_eq!(err, PublishErrorV1::StaleUpstream { artifact: upstream });
    }

    #[test]
    fn same_story_bytes_can_miss_when_environment_dependency_changes() {
        let text = b"hello";
        let env_a = b"font-a";
        let env_b = b"font-b";

        let a = fingerprint_v1("prepared-input-v1", &[text, env_a]);
        let b = fingerprint_v1("prepared-input-v1", &[text, env_b]);

        assert_ne!(a, b);
    }

    #[test]
    fn canonical_fingerprint_is_domain_separated_and_ordered() {
        let a = fingerprint_v1("stage-a", &[b"x", b"y"]);
        let b = fingerprint_v1("stage-b", &[b"x", b"y"]);
        let c = fingerprint_v1("stage-a", &[b"xy"]);
        let d = fingerprint_v1("stage-a", &[b"y", b"x"]);

        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }
}
