use pub_core::RawSpan;
use serde::{Deserialize, Serialize};

mod mcld;
mod story;
mod tokn;
mod typography;
mod writer;

pub use mcld::{
    QuillMcldChild, QuillMcldChunk, QuillMcldConsensusU32, QuillMcldField, QuillMcldFieldValue,
    QuillMcldReadError, QuillMcldRecord, QuillMcldTableMetrics, bounded_mcld_table_metrics,
    parse_bounded_mcld,
};
pub use story::{
    QUILL_DESCRIPTOR_LIST_END, QUILL_DESCRIPTOR_LIST_ROOT_OFFSET, QUILL_DESCRIPTOR_PRESENCE_MARKER,
    QUILL_DESCRIPTOR_SIZE, QuillChunkDescriptor, QuillDescriptorListNode, QuillStoryCatalog,
    QuillStoryReadError, QuillStorySlice, QuillStrsChunk, QuillSyidChunk, QuillTcdChunk,
    QuillTextChunk, parse_confirmed_story_catalog,
};
pub use writer::{
    QuillStoryTextEdit, QuillStoryTextWritePlan, QuillStoryWriteError, plan_quill_story_text_edit,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuillChunk {
    pub name: String,
    pub source: RawSpan,
    pub payload: Vec<u8>,
}

pub use tokn::{
    QuillToknChunk, QuillToknEffectiveToken, QuillToknProperty, QuillToknPropertyBlock,
    QuillToknTargetRecord, QuillToknTargetSection, QuillToknTargetSectionHeader, TOKN_PLC_TYPE,
    TOKN_PROPERTY_KIND, TOKN_PROPERTY_STATE, TOKN_PROPERTY_TEXT_LENGTH,
};
pub use typography::{
    QUILL_TEXT_SIZE_EMU_PER_POINT, QuillExplicitTypographyRun, QuillTypographyCatalog,
    QuillTypographyRange, QuillTypographyReadError, QuillTypographyStoryIntersection,
    parse_bounded_typography,
};
