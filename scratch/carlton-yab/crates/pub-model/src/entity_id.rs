use crate::CanonicalId;
use serde::{Deserialize, Serialize};

macro_rules! canonical_id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(CanonicalId);

        impl $name {
            pub const fn from_canonical(id: CanonicalId) -> Self {
                Self(id)
            }

            pub const fn as_canonical(&self) -> &CanonicalId {
                &self.0
            }

            pub const fn into_canonical(self) -> CanonicalId {
                self.0
            }
        }

        impl From<CanonicalId> for $name {
            fn from(id: CanonicalId) -> Self {
                Self(id)
            }
        }

        impl From<$name> for CanonicalId {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

canonical_id_type!(DocumentId, "Canonical identity документа.");
canonical_id_type!(PageId, "Canonical identity authored page.");
canonical_id_type!(NodeId, "Canonical identity visual/placed node.");
canonical_id_type!(
    TableCellId,
    "Canonical identity authored table cell, separate from visual Node identity."
);
canonical_id_type!(StoryId, "Canonical identity logical text Story.");
canonical_id_type!(ParagraphId, "Canonical identity paragraph.");
canonical_id_type!(TextRunId, "Canonical identity character/text run.");
canonical_id_type!(ResourceId, "Canonical identity shared resource.");
canonical_id_type!(StyleId, "Canonical identity style definition.");
canonical_id_type!(ExtensionId, "Canonical identity opaque extension.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_ids_keep_same_wire_representation_as_canonical_id() {
        let canonical = CanonicalId::from_bytes([0x11; 16]);
        let page = PageId::from_canonical(canonical);

        assert_eq!(page.into_canonical(), canonical);
        assert_eq!(
            serde_json::to_value(page).expect("PageId должен сериализоваться"),
            serde_json::to_value(canonical).expect("CanonicalId должен сериализоваться")
        );
    }

    #[test]
    fn different_semantic_id_types_do_not_need_distinct_wire_formats() {
        let canonical = CanonicalId::from_bytes([0x22; 16]);
        let page = PageId::from_canonical(canonical);
        let story = StoryId::from_canonical(canonical);

        assert_eq!(page.as_canonical(), story.as_canonical());
    }
}
