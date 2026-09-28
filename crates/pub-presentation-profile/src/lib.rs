use pub_model::derive_pub_page_id_v1;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

pub const CARLTON_PRESENTATION_INPUT_SCHEMA_V1: &str =
    "chaptera.carlton-presentation-profile-input.v1";
pub const CARLTON_PRESENTATION_MANIFEST_SCHEMA_V1: &str =
    "chaptera.carlton-presentation-manifest.v1";

const MARCH_2026_SHA256: &str = "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3";
const DECEMBER_2025_SHA256: &str =
    "41786e9ee564dfc3d10864a49c9e58af5e1689d31479ef0c478ffb79da16f79e";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarltonPageEvidenceV1 {
    pub document_ordinal: usize,
    pub contents_seq_num: u32,
    pub oid_dword0: Option<u32>,
    pub oid_dword1: Option<u32>,
    pub applied_master_seq_num: Option<u32>,
    pub shape_child_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarltonPresentationProfileInputV1 {
    pub schema_version: String,
    pub source_sha256: String,
    pub pages: Vec<CarltonPageEvidenceV1>,
    #[serde(default)]
    pub carrier_page_seq_nums: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarltonPageRoleV1 {
    Customer,
    Master,
    Carrier,
    InternalService,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarltonPageRoleReceiptV1 {
    pub document_ordinal: usize,
    pub contents_seq_num: u32,
    pub page_id: String,
    pub oid_dword0: Option<u32>,
    pub oid_dword1: Option<u32>,
    pub applied_master_seq_num: Option<u32>,
    pub shape_child_count: usize,
    pub role: CarltonPageRoleV1,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarltonMasterPresentationRelationV1 {
    pub source_page_seq_num: u32,
    pub source_page_id: String,
    pub master_page_seq_num: u32,
    pub master_page_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarltonPresentationInvariantsV1 {
    pub exact_source_admission: bool,
    pub family_scoped_oid_rule: bool,
    pub canonical_source_graph_mutated: bool,
    pub canonical_page_ids_preserved: bool,
    pub carrier_pages_exposed_as_customer_pages: bool,
    pub master_pages_exposed_as_customer_pages: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarltonPresentationManifestV1 {
    pub schema_version: String,
    pub profile_id: String,
    pub source_sha256: String,
    pub raw_page_count: usize,
    pub customer_page_count: usize,
    pub customer_page_seq_nums: Vec<u32>,
    pub customer_page_ids: Vec<String>,
    pub master_page_seq_nums: Vec<u32>,
    pub carrier_page_seq_nums: Vec<u32>,
    pub pages: Vec<CarltonPageRoleReceiptV1>,
    pub customer_master_relations: Vec<CarltonMasterPresentationRelationV1>,
    pub invariants: CarltonPresentationInvariantsV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarltonPresentationError {
    SchemaVersionMismatch,
    UnsupportedSourceHash,
    EmptyPageSet,
    DuplicateDocumentOrdinal(usize),
    DuplicatePageSeqNum(u32),
    CustomerCountMismatch {
        expected: usize,
        observed: usize,
    },
    CustomerPageSetMismatch {
        expected: Vec<u32>,
        observed: Vec<u32>,
    },
    MissingMasterTarget,
    MultipleMasterTargets(Vec<u32>),
    MasterTargetMissingFromPages(u32),
    MasterLooksCustomerVisible(u32),
    UnknownCarrierPage(u32),
    CarrierCustomerOverlap(u32),
    CarrierMasterOverlap(u32),
    CustomerMissingExpectedMaster {
        page_seq_num: u32,
        expected_master: u32,
    },
    IdentityDerivation {
        page_seq_num: u32,
    },
}

impl fmt::Display for CarltonPresentationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaVersionMismatch => write!(f, "Carlton presentation input schema mismatch"),
            Self::UnsupportedSourceHash => {
                write!(f, "source is not an admitted Carlton family control")
            }
            Self::EmptyPageSet => write!(f, "Carlton presentation input has no PAGE evidence"),
            Self::DuplicateDocumentOrdinal(value) => {
                write!(f, "duplicate Carlton document ordinal {value}")
            }
            Self::DuplicatePageSeqNum(value) => {
                write!(f, "duplicate Carlton PAGE seqNum {value}")
            }
            Self::CustomerCountMismatch { expected, observed } => write!(
                f,
                "Carlton customer-page count mismatch: expected {expected}, observed {observed}"
            ),
            Self::CustomerPageSetMismatch { expected, observed } => write!(
                f,
                "Carlton customer-page set mismatch: expected {expected:?}, observed {observed:?}"
            ),
            Self::MissingMasterTarget => {
                write!(f, "Carlton family evidence has no applied-master target")
            }
            Self::MultipleMasterTargets(values) => {
                write!(
                    f,
                    "Carlton family evidence has multiple master targets: {values:?}"
                )
            }
            Self::MasterTargetMissingFromPages(value) => {
                write!(
                    f,
                    "Carlton master target PAGE {value} is absent from PAGE evidence"
                )
            }
            Self::MasterLooksCustomerVisible(value) => write!(
                f,
                "Carlton master PAGE {value} unexpectedly satisfies the family customer Oid rule"
            ),
            Self::UnknownCarrierPage(value) => {
                write!(
                    f,
                    "Carlton Cmo carrier PAGE {value} is absent from PAGE evidence"
                )
            }
            Self::CarrierCustomerOverlap(value) => write!(
                f,
                "Carlton Cmo carrier PAGE {value} also satisfies the customer-page rule"
            ),
            Self::CarrierMasterOverlap(value) => {
                write!(
                    f,
                    "Carlton Cmo carrier PAGE {value} is also the master PAGE"
                )
            }
            Self::CustomerMissingExpectedMaster {
                page_seq_num,
                expected_master,
            } => write!(
                f,
                "Carlton customer PAGE {page_seq_num} does not apply expected master PAGE {expected_master}"
            ),
            Self::IdentityDerivation { page_seq_num } => write!(
                f,
                "cannot derive canonical PageId for Carlton PAGE {page_seq_num}"
            ),
        }
    }
}

impl Error for CarltonPresentationError {}

#[derive(Debug, Clone, Copy)]
struct AdmittedProfile {
    profile_id: &'static str,
    expected_customer_seq_nums: &'static [u32],
    carrier_page_seq_nums: &'static [u32],
}

fn admitted_profile(source_sha256: &str) -> Result<AdmittedProfile, CarltonPresentationError> {
    match source_sha256 {
        MARCH_2026_SHA256 => Ok(AdmittedProfile {
            profile_id: "carlton-school-jotter/march-2026/v1",
            expected_customer_seq_nums: &[266, 361, 406],
            carrier_page_seq_nums: &[279],
        }),
        DECEMBER_2025_SHA256 => Ok(AdmittedProfile {
            profile_id: "carlton-school-jotter/december-2025/v1",
            expected_customer_seq_nums: &[266, 361, 406, 456, 493],
            carrier_page_seq_nums: &[279],
        }),
        _ => Err(CarltonPresentationError::UnsupportedSourceHash),
    }
}

pub fn build_admitted_carlton_presentation_manifest_v1(
    source_sha256: &str,
    pages: Vec<CarltonPageEvidenceV1>,
) -> Result<CarltonPresentationManifestV1, CarltonPresentationError> {
    let profile = admitted_profile(source_sha256)?;
    build_carlton_presentation_manifest_v1(CarltonPresentationProfileInputV1 {
        schema_version: CARLTON_PRESENTATION_INPUT_SCHEMA_V1.to_owned(),
        source_sha256: source_sha256.to_owned(),
        pages,
        carrier_page_seq_nums: profile.carrier_page_seq_nums.to_vec(),
    })
}

pub fn build_carlton_presentation_manifest_v1(
    mut input: CarltonPresentationProfileInputV1,
) -> Result<CarltonPresentationManifestV1, CarltonPresentationError> {
    if input.schema_version != CARLTON_PRESENTATION_INPUT_SCHEMA_V1 {
        return Err(CarltonPresentationError::SchemaVersionMismatch);
    }
    let profile = admitted_profile(&input.source_sha256)?;
    if input.pages.is_empty() {
        return Err(CarltonPresentationError::EmptyPageSet);
    }

    input.pages.sort_by_key(|page| page.document_ordinal);
    let mut ordinals = BTreeSet::new();
    let mut pages_by_seq = BTreeMap::new();
    for page in &input.pages {
        if !ordinals.insert(page.document_ordinal) {
            return Err(CarltonPresentationError::DuplicateDocumentOrdinal(
                page.document_ordinal,
            ));
        }
        if pages_by_seq.insert(page.contents_seq_num, page).is_some() {
            return Err(CarltonPresentationError::DuplicatePageSeqNum(
                page.contents_seq_num,
            ));
        }
    }

    let customer_seq_nums = input
        .pages
        .iter()
        .filter(|page| page.oid_dword0 == Some(2))
        .map(|page| page.contents_seq_num)
        .collect::<Vec<_>>();
    if customer_seq_nums.len() != profile.expected_customer_seq_nums.len() {
        return Err(CarltonPresentationError::CustomerCountMismatch {
            expected: profile.expected_customer_seq_nums.len(),
            observed: customer_seq_nums.len(),
        });
    }
    if customer_seq_nums != profile.expected_customer_seq_nums {
        return Err(CarltonPresentationError::CustomerPageSetMismatch {
            expected: profile.expected_customer_seq_nums.to_vec(),
            observed: customer_seq_nums,
        });
    }
    let customer_set = customer_seq_nums.iter().copied().collect::<BTreeSet<_>>();

    let master_targets = input
        .pages
        .iter()
        .filter_map(|page| page.applied_master_seq_num)
        .collect::<BTreeSet<_>>();
    let master_seq = match master_targets.len() {
        0 => return Err(CarltonPresentationError::MissingMasterTarget),
        1 => *master_targets.first().expect("one master"),
        _ => {
            return Err(CarltonPresentationError::MultipleMasterTargets(
                master_targets.iter().copied().collect(),
            ));
        }
    };
    let master_page = pages_by_seq.get(&master_seq).copied().ok_or(
        CarltonPresentationError::MasterTargetMissingFromPages(master_seq),
    )?;
    if master_page.oid_dword0 == Some(2) {
        return Err(CarltonPresentationError::MasterLooksCustomerVisible(
            master_seq,
        ));
    }

    let mut carrier_set = BTreeSet::new();
    for seq_num in input.carrier_page_seq_nums {
        if !pages_by_seq.contains_key(&seq_num) {
            return Err(CarltonPresentationError::UnknownCarrierPage(seq_num));
        }
        if customer_set.contains(&seq_num) {
            return Err(CarltonPresentationError::CarrierCustomerOverlap(seq_num));
        }
        if seq_num == master_seq {
            return Err(CarltonPresentationError::CarrierMasterOverlap(seq_num));
        }
        carrier_set.insert(seq_num);
    }

    for seq_num in &customer_seq_nums {
        let page = pages_by_seq[seq_num];
        if page.applied_master_seq_num != Some(master_seq) {
            return Err(CarltonPresentationError::CustomerMissingExpectedMaster {
                page_seq_num: *seq_num,
                expected_master: master_seq,
            });
        }
    }

    let page_id = |seq_num: u32| {
        derive_pub_page_id_v1(&input.source_sha256, seq_num).map_err(|_| {
            CarltonPresentationError::IdentityDerivation {
                page_seq_num: seq_num,
            }
        })
    };

    let mut pages = Vec::with_capacity(input.pages.len());
    for page in &input.pages {
        let role = if page.contents_seq_num == master_seq {
            CarltonPageRoleV1::Master
        } else if carrier_set.contains(&page.contents_seq_num) {
            CarltonPageRoleV1::Carrier
        } else if customer_set.contains(&page.contents_seq_num) {
            CarltonPageRoleV1::Customer
        } else {
            CarltonPageRoleV1::InternalService
        };
        let reason_codes = match role {
            CarltonPageRoleV1::Customer => vec![
                "exact_family_admitted".to_owned(),
                "family_oid_dword0_2".to_owned(),
                "expected_master_applied".to_owned(),
            ],
            CarltonPageRoleV1::Master => vec![
                "exact_family_admitted".to_owned(),
                "referenced_as_master_target".to_owned(),
            ],
            CarltonPageRoleV1::Carrier => vec![
                "exact_family_admitted".to_owned(),
                "plccmob_carrier_parent".to_owned(),
            ],
            CarltonPageRoleV1::InternalService => vec![
                "exact_family_admitted".to_owned(),
                "residual_non_customer_page".to_owned(),
            ],
        };
        pages.push(CarltonPageRoleReceiptV1 {
            document_ordinal: page.document_ordinal,
            contents_seq_num: page.contents_seq_num,
            page_id: page_id(page.contents_seq_num)?,
            oid_dword0: page.oid_dword0,
            oid_dword1: page.oid_dword1,
            applied_master_seq_num: page.applied_master_seq_num,
            shape_child_count: page.shape_child_count,
            role,
            reason_codes,
        });
    }

    let customer_page_ids = customer_seq_nums
        .iter()
        .map(|seq_num| page_id(*seq_num))
        .collect::<Result<Vec<_>, _>>()?;
    let master_page_id = page_id(master_seq)?;
    let customer_master_relations = customer_seq_nums
        .iter()
        .map(|seq_num| {
            Ok(CarltonMasterPresentationRelationV1 {
                source_page_seq_num: *seq_num,
                source_page_id: page_id(*seq_num)?,
                master_page_seq_num: master_seq,
                master_page_id: master_page_id.clone(),
            })
        })
        .collect::<Result<Vec<_>, CarltonPresentationError>>()?;

    Ok(CarltonPresentationManifestV1 {
        schema_version: CARLTON_PRESENTATION_MANIFEST_SCHEMA_V1.to_owned(),
        profile_id: profile.profile_id.to_owned(),
        source_sha256: input.source_sha256,
        raw_page_count: input.pages.len(),
        customer_page_count: customer_seq_nums.len(),
        customer_page_seq_nums: customer_seq_nums,
        customer_page_ids,
        master_page_seq_nums: vec![master_seq],
        carrier_page_seq_nums: carrier_set.into_iter().collect(),
        pages,
        customer_master_relations,
        invariants: CarltonPresentationInvariantsV1 {
            exact_source_admission: true,
            family_scoped_oid_rule: true,
            canonical_source_graph_mutated: false,
            canonical_page_ids_preserved: true,
            carrier_pages_exposed_as_customer_pages: false,
            master_pages_exposed_as_customer_pages: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn march_input() -> CarltonPresentationProfileInputV1 {
        let seqs = [263, 266, 361, 406, 269, 272, 275, 279];
        let customer = BTreeMap::from([(266, (2, 0)), (361, (2, 5)), (406, (2, 3))]);
        CarltonPresentationProfileInputV1 {
            schema_version: CARLTON_PRESENTATION_INPUT_SCHEMA_V1.to_owned(),
            source_sha256: MARCH_2026_SHA256.to_owned(),
            pages: seqs
                .into_iter()
                .enumerate()
                .map(
                    |(document_ordinal, contents_seq_num)| CarltonPageEvidenceV1 {
                        document_ordinal,
                        contents_seq_num,
                        oid_dword0: customer
                            .get(&contents_seq_num)
                            .map(|value| value.0)
                            .or(Some(0)),
                        oid_dword1: customer
                            .get(&contents_seq_num)
                            .map(|value| value.1)
                            .or(Some(0)),
                        applied_master_seq_num: (contents_seq_num != 263).then_some(263),
                        shape_child_count: match contents_seq_num {
                            263 => 2,
                            266 => 23,
                            361 => 34,
                            406 => 14,
                            279 => 9,
                            _ => 0,
                        },
                    },
                )
                .collect(),
            carrier_page_seq_nums: vec![279],
        }
    }

    #[test]
    fn march_profile_classifies_three_customer_pages_without_mutating_source_truth() {
        let manifest = build_carlton_presentation_manifest_v1(march_input()).unwrap();
        assert_eq!(manifest.customer_page_seq_nums, vec![266, 361, 406]);
        assert_eq!(manifest.master_page_seq_nums, vec![263]);
        assert_eq!(manifest.carrier_page_seq_nums, vec![279]);
        assert_eq!(
            manifest
                .pages
                .iter()
                .map(|page| (page.contents_seq_num, page.role))
                .collect::<Vec<_>>(),
            vec![
                (263, CarltonPageRoleV1::Master),
                (266, CarltonPageRoleV1::Customer),
                (361, CarltonPageRoleV1::Customer),
                (406, CarltonPageRoleV1::Customer),
                (269, CarltonPageRoleV1::InternalService),
                (272, CarltonPageRoleV1::InternalService),
                (275, CarltonPageRoleV1::InternalService),
                (279, CarltonPageRoleV1::Carrier),
            ]
        );
        assert!(!manifest.invariants.canonical_source_graph_mutated);
        assert!(manifest.invariants.canonical_page_ids_preserved);
    }

    #[test]
    fn applied_master_presence_is_not_a_customer_classifier() {
        let manifest = build_carlton_presentation_manifest_v1(march_input()).unwrap();
        for seq_num in [269, 272, 275, 279] {
            let page = manifest
                .pages
                .iter()
                .find(|page| page.contents_seq_num == seq_num)
                .unwrap();
            assert_eq!(page.applied_master_seq_num, Some(263));
            assert_ne!(page.role, CarltonPageRoleV1::Customer);
        }
    }

    #[test]
    fn unknown_source_hash_fails_closed_before_oid_rule_is_applied() {
        let mut input = march_input();
        input.source_sha256 = "11".repeat(32);
        assert_eq!(
            build_carlton_presentation_manifest_v1(input),
            Err(CarltonPresentationError::UnsupportedSourceHash)
        );
    }

    #[test]
    fn family_drift_in_customer_count_fails_closed() {
        let mut input = march_input();
        input
            .pages
            .iter_mut()
            .find(|page| page.contents_seq_num == 269)
            .unwrap()
            .oid_dword0 = Some(2);
        assert_eq!(
            build_carlton_presentation_manifest_v1(input),
            Err(CarltonPresentationError::CustomerCountMismatch {
                expected: 3,
                observed: 4,
            })
        );
    }

    #[test]
    fn admitted_runtime_helper_reuses_exact_family_profile() {
        let input = march_input();
        let manifest = build_admitted_carlton_presentation_manifest_v1(
            &input.source_sha256,
            input.pages,
        )
        .unwrap();
        assert_eq!(manifest.customer_page_seq_nums, vec![266, 361, 406]);
        assert_eq!(manifest.carrier_page_seq_nums, vec![279]);
    }

    #[test]
    fn same_count_wrong_customer_set_fails_closed() {
        let mut input = march_input();
        input
            .pages
            .iter_mut()
            .find(|page| page.contents_seq_num == 406)
            .unwrap()
            .oid_dword0 = Some(0);
        input
            .pages
            .iter_mut()
            .find(|page| page.contents_seq_num == 269)
            .unwrap()
            .oid_dword0 = Some(2);

        assert_eq!(
            build_carlton_presentation_manifest_v1(input),
            Err(CarltonPresentationError::CustomerPageSetMismatch {
                expected: vec![266, 361, 406],
                observed: vec![266, 361, 269],
            })
        );
    }

    #[test]
    fn carrier_evidence_cannot_overlap_customer_or_master_pages() {
        let mut input = march_input();
        input.carrier_page_seq_nums = vec![266];
        assert_eq!(
            build_carlton_presentation_manifest_v1(input),
            Err(CarltonPresentationError::CarrierCustomerOverlap(266))
        );

        let mut input = march_input();
        input.carrier_page_seq_nums = vec![263];
        assert_eq!(
            build_carlton_presentation_manifest_v1(input),
            Err(CarltonPresentationError::CarrierMasterOverlap(263))
        );
    }
}
