use super::{
    CONTENTS_STREAM_PATH, PUB_ADAPTER_ID, PubBridgeDiagnostic, PubEffectivePageProjection,
    PubEffectivePageProjectionAuthority, PubExplicitShapePaintSource, PubNodePayload,
    PubSourceGraph, PubSourceGraphBuild, PubStoryFrameSource, QUILL_STREAM_PATH, ROLE_DOCUMENT,
    ROLE_NODE, ROLE_PAGE, decode_utf16le_strict, derive_pub_id, quill_story_object_key, source_ref,
};
use anyhow::{Context, Result, bail};
use pub_contents::{Legacy0x22Directory, Legacy0x22DirectoryEntry, parse_legacy_0x22_directory};
use pub_core::{RawSpan, StreamPath};
use pub_model::{
    Affine2D, AuthorityClass, Document, DocumentId, LengthEmu, Node, NodeHeader, NodeId, NodeKind,
    Page, PageId, ReadConfidence, RectEmu, Sha256Digest, Size2D, SourceDescriptor, SourceRole,
    SourceGraph, Story, StoryId,
};
use pub_quill::parse_confirmed_story_catalog;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek, SeekFrom};

const LEGACY_DOCUMENT_TYPE: u16 = 0x0015;
const LEGACY_PAGE_TYPE: u16 = 0x0014;
const LEGACY_TEXT_SHAPE_TYPE: u16 = 0x0008;
const LEGACY_LIST_HEADER_SIZE: usize = 10;
const LEGACY_LIST_U16_RECORD_SIZE: u16 = 2;
const LEGACY_DOCUMENT_WIDTH_OFFSET: usize = 0x14;
const LEGACY_DOCUMENT_HEIGHT_OFFSET: usize = 0x18;
const LEGACY_SHAPE_XS_OFFSET: usize = 0x06;
const LEGACY_SHAPE_YS_OFFSET: usize = 0x0a;
const LEGACY_SHAPE_XE_OFFSET: usize = 0x0e;
const LEGACY_SHAPE_YE_OFFSET: usize = 0x12;
const LEGACY_TEXT_ID_OFFSET: usize = 0x58;

#[derive(Debug, Clone)]
struct LegacyIdList {
    ids: Vec<(u16, RawSpan)>,
}

pub fn legacy22_object_key(object_id: u16) -> String {
    format!("contents/0x22/object/{object_id}")
}

fn derive_legacy_document_id(
    source_hash: &Sha256Digest,
    object_id: u16,
) -> Result<DocumentId> {
    Ok(DocumentId::from_canonical(derive_pub_id(
        source_hash,
        &legacy22_object_key(object_id),
        ROLE_DOCUMENT,
    )?))
}

fn derive_legacy_page_id(source_hash: &Sha256Digest, object_id: u16) -> Result<PageId> {
    Ok(PageId::from_canonical(derive_pub_id(
        source_hash,
        &legacy22_object_key(object_id),
        ROLE_PAGE,
    )?))
}

fn derive_legacy_node_id(source_hash: &Sha256Digest, object_id: u16) -> Result<NodeId> {
    Ok(NodeId::from_canonical(derive_pub_id(
        source_hash,
        &legacy22_object_key(object_id),
        ROLE_NODE,
    )?))
}

pub fn build_legacy_0x22_quill_source_graph<R: Read + Seek>(
    mut reader: R,
    source_hash: Sha256Digest,
) -> Result<PubSourceGraphBuild> {
    reader.seek(SeekFrom::Start(0))?;
    let mut pub_bytes = Vec::new();
    reader.read_to_end(&mut pub_bytes)?;

    let contents =
        pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), CONTENTS_STREAM_PATH)
            .with_context(|| format!("read {CONTENTS_STREAM_PATH}"))?;
    let quill = pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), QUILL_STREAM_PATH)
        .with_context(|| format!("read {QUILL_STREAM_PATH}"))?;

    build_legacy_0x22_quill_from_streams(source_hash, &contents, &quill)
}

pub fn build_legacy_0x22_quill_from_streams(
    source_hash: Sha256Digest,
    contents: &[u8],
    quill: &[u8],
) -> Result<PubSourceGraphBuild> {
    let adapter_version = format!("pub-rs/{}", env!("CARGO_PKG_VERSION"));
    let source = SourceDescriptor {
        format: "pub".into(),
        format_version: Some("0x22-quill".into()),
        adapter_version,
        source_hash,
    };
    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let directory = parse_legacy_0x22_directory(contents_stream.clone(), contents)
        .context("parse legacy 0x22 Contents directory")?;

    let document_entry =
        unique_entry_by_type(&directory, LEGACY_DOCUMENT_TYPE, "DOCUMENT 0x0015")?;
    let document_object_id = document_entry.object_id;
    let document_chunk = chunk_bytes(contents, document_entry)?;
    let page_width_emu = read_u32(document_chunk, LEGACY_DOCUMENT_WIDTH_OFFSET)
        .context("legacy DOCUMENT width is missing")?;
    let page_height_emu = read_u32(document_chunk, LEGACY_DOCUMENT_HEIGHT_OFFSET)
        .context("legacy DOCUMENT height is missing")?;
    if page_width_emu == 0 || page_height_emu == 0 {
        bail!(
            "legacy DOCUMENT has non-positive page extent {}x{}",
            page_width_emu,
            page_height_emu
        );
    }
    let document_page_list = parse_u16_id_list(contents, document_entry, "DOCUMENT PageList")?;
    if document_page_list.ids.is_empty() {
        bail!("legacy DOCUMENT PageList is empty");
    }

    let document_id = derive_legacy_document_id(&source_hash, document_object_id)?;
    let mut document_pages = Vec::with_capacity(document_page_list.ids.len());
    let mut pages = BTreeMap::new();
    let mut page_object_to_id = BTreeMap::new();
    let mut seen_pages = BTreeSet::new();

    for (page_object_id, _) in &document_page_list.ids {
        if !seen_pages.insert(*page_object_id) {
            bail!("legacy DOCUMENT PageList repeats object {page_object_id}");
        }
        let entry = directory
            .entry_by_object_id(*page_object_id)
            .with_context(|| format!("legacy DOCUMENT PageList references missing object {page_object_id}"))?;
        if entry.chunk_type != LEGACY_PAGE_TYPE {
            bail!(
                "legacy DOCUMENT PageList object {} has type {:#06x}, expected PAGE 0x0014",
                page_object_id,
                entry.chunk_type
            );
        }
        let page_id = derive_legacy_page_id(&source_hash, *page_object_id)?;
        document_pages.push(page_id);
        page_object_to_id.insert(*page_object_id, page_id);
        pages.insert(
            page_id,
            Page {
                id: page_id,
                size: Size2D::new(
                    LengthEmu::new(i64::from(page_width_emu)),
                    LengthEmu::new(i64::from(page_height_emu)),
                ),
                bleed: None,
                margins: None,
                children: Vec::new(),
                extensions: Vec::new(),
            },
        );
    }

    let document = Document {
        id: document_id,
        format_origin: "pub".into(),
        source_hash,
        pages: document_pages.clone(),
        resources: Vec::new(),
        styles: Vec::new(),
    };
    let mut graph = PubSourceGraph::empty(source, document);
    graph.pages = pages;

    let quill_stream = StreamPath(QUILL_STREAM_PATH.into());
    let quill_catalog = parse_confirmed_story_catalog(quill_stream, quill)
        .context("parse legacy 0x22 Quill story catalog")?;
    let mut story_by_syid = BTreeMap::new();
    for story_slice in &quill_catalog.stories {
        let syid = story_slice.syid.0;
        let story_id = super::derive_pub_story_id(&source_hash, syid)?;
        let object_key = quill_story_object_key(syid);
        let text = decode_utf16le_strict(&story_slice.utf16le)
            .with_context(|| format!("decode legacy Quill Story SYID {syid} as UTF-16LE"))?;
        let source_refs = vec![
            source_ref(
                &graph.source,
                &story_slice.syid_source,
                Some(object_key.clone()),
                Some("SYID".into()),
                SourceRole::Relation,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                &graph.source,
                &story_slice.text_source,
                Some(object_key),
                Some("TEXT".into()),
                SourceRole::Semantic,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
        ];
        graph.stories.insert(
            story_id,
            Story {
                id: story_id,
                text,
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs,
            },
        );
        story_by_syid.insert(syid, story_id);
    }

    let mut diagnostics = Vec::new();
    for (page_object_id, page_id) in &page_object_to_id {
        let page_entry = directory
            .entry_by_object_id(*page_object_id)
            .expect("PageList entry was verified above");
        let child_list = parse_u16_id_list(contents, page_entry, "PAGE child list")
            .with_context(|| format!("parse legacy PAGE {page_object_id} child list"))?;
        let page = graph
            .pages
            .get(page_id)
            .expect("verified page must be in SourceGraph")
            .clone();

        for (child_object_id, _) in child_list.ids {
            let Some(child_entry) = directory.entry_by_object_id(child_object_id) else {
                diagnostics.push(PubBridgeDiagnostic::LegacyObjectNotMaterialized {
                    object_id: u32::from(child_object_id),
                    raw_type: None,
                    reason: "page_child_missing_from_directory".into(),
                });
                continue;
            };
            if child_entry.parent_id != *page_object_id {
                diagnostics.push(PubBridgeDiagnostic::LegacyObjectNotMaterialized {
                    object_id: u32::from(child_object_id),
                    raw_type: Some(child_entry.chunk_type),
                    reason: "page_child_parent_mismatch".into(),
                });
                continue;
            }
            if child_entry.chunk_type != LEGACY_TEXT_SHAPE_TYPE {
                diagnostics.push(PubBridgeDiagnostic::LegacyObjectNotMaterialized {
                    object_id: u32::from(child_object_id),
                    raw_type: Some(child_entry.chunk_type),
                    reason: "legacy_object_type_not_admitted_v1".into(),
                });
                continue;
            }

            let chunk = chunk_bytes(contents, child_entry)?;
            let Some(bounds) = legacy_text_shape_bounds(&page, chunk) else {
                diagnostics.push(PubBridgeDiagnostic::LegacyObjectNotMaterialized {
                    object_id: u32::from(child_object_id),
                    raw_type: Some(child_entry.chunk_type),
                    reason: "invalid_or_incomplete_geometry".into(),
                });
                continue;
            };
            let text_id = read_u16(chunk, LEGACY_TEXT_ID_OFFSET)
                .with_context(|| format!("legacy TEXT SHAPE {child_object_id} textId missing"))?;
            let story_id = story_by_syid.get(&u32::from(text_id)).copied();
            if story_id.is_none() {
                diagnostics.push(PubBridgeDiagnostic::MissingQuillStory {
                    seq_num: u32::from(child_object_id),
                    text_id: u32::from(text_id),
                });
            }

            let node_id = derive_legacy_node_id(&source_hash, child_object_id)?;
            let object_key = legacy22_object_key(child_object_id);
            let geometry_span = RawSpan {
                stream: contents_stream.clone(),
                offset: child_entry.chunk_source.offset + LEGACY_SHAPE_XS_OFFSET as u64,
                len: 16,
            };
            let source_refs = vec![
                source_ref(
                    &graph.source,
                    &child_entry.entry_source,
                    Some(object_key.clone()),
                    Some("directory_entry".into()),
                    SourceRole::Relation,
                    AuthorityClass::Authoritative,
                    ReadConfidence::Exact,
                ),
                source_ref(
                    &graph.source,
                    &geometry_span,
                    Some(object_key),
                    Some("legacy_center_origin_geometry".into()),
                    SourceRole::Projection,
                    AuthorityClass::Authoritative,
                    ReadConfidence::Exact,
                ),
            ];

            graph.nodes.insert(
                node_id,
                Node {
                    kind: NodeKind::Shape,
                    header: NodeHeader {
                        id: node_id,
                        parent_id: page_id.into_canonical(),
                        bounds,
                        transform: Affine2D::identity(),
                        source_refs,
                        extensions: Vec::new(),
                    },
                    payload: PubNodePayload {
                        contents_seq_num: u32::from(child_object_id),
                        officeart_shape_type: None,
                        officeart_spid: None,
                        image_slot: None,
                        explicit_image_crop: None,
                        explicit_paint: PubExplicitShapePaintSource::default(),
                        story_frame: Some(PubStoryFrameSource {
                            text_id: u32::from(text_id),
                            story_id,
                            explicit_ordinal: None,
                            previous_seq_num: None,
                            previous_frame: None,
                            next_seq_num: None,
                            next_frame: None,
                        }),
                        table_story: None,
                        table: None,
                    },
                },
            );
        }
    }

    let effective_pages = PubEffectivePageProjection {
        authority: PubEffectivePageProjectionAuthority::RawDocumentPageList,
        page_ids: document_pages.clone(),
        raw_page_count: document_pages.len(),
        observed_scenario_page_ids: Vec::new(),
        scenario_evidence_list_count: 0,
    };

    Ok(PubSourceGraphBuild {
        graph,
        effective_pages,
        diagnostics,
        typography_runs: Vec::new(),
    })
}

fn unique_entry_by_type<'a>(
    directory: &'a Legacy0x22Directory,
    chunk_type: u16,
    label: &str,
) -> Result<&'a Legacy0x22DirectoryEntry> {
    let mut matches = directory
        .entries
        .iter()
        .filter(|entry| entry.chunk_type == chunk_type);
    let first = matches
        .next()
        .with_context(|| format!("missing legacy {label}"))?;
    if matches.next().is_some() {
        bail!("multiple legacy {label} objects");
    }
    Ok(first)
}

fn chunk_bytes<'a>(
    contents: &'a [u8],
    entry: &Legacy0x22DirectoryEntry,
) -> Result<&'a [u8]> {
    let start = usize::try_from(entry.chunk_source.offset)
        .context("legacy chunk offset does not fit usize")?;
    let len = usize::try_from(entry.chunk_source.len)
        .context("legacy chunk length does not fit usize")?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= contents.len())
        .context("legacy chunk span exceeds Contents")?;
    Ok(&contents[start..end])
}

fn parse_u16_id_list(
    contents: &[u8],
    entry: &Legacy0x22DirectoryEntry,
    label: &str,
) -> Result<LegacyIdList> {
    let chunk_start = usize::try_from(entry.chunk_source.offset)
        .context("legacy chunk offset does not fit usize")?;
    let chunk_len = usize::try_from(entry.chunk_source.len)
        .context("legacy chunk length does not fit usize")?;
    let chunk_end = chunk_start
        .checked_add(chunk_len)
        .filter(|end| *end <= contents.len())
        .context("legacy chunk span exceeds Contents")?;
    let data_delta = *contents
        .get(chunk_start + 3)
        .with_context(|| format!("{label}: missing dataRelativeOffset"))?;
    let list_start = chunk_start
        .checked_add(usize::from(data_delta))
        .context("legacy list offset overflow")?;
    let header_end = list_start
        .checked_add(LEGACY_LIST_HEADER_SIZE)
        .context("legacy list header overflow")?;
    if header_end > chunk_end {
        bail!("{label}: list header exceeds bounded chunk");
    }

    let count = read_u16(contents, list_start).expect("list header bounds checked");
    let max_count = read_u16(contents, list_start + 2).expect("list header bounds checked");
    let record_size = read_u16(contents, list_start + 4).expect("list header bounds checked");
    if max_count < count {
        bail!("{label}: max_count {max_count} is less than count {count}");
    }
    if record_size != LEGACY_LIST_U16_RECORD_SIZE {
        bail!(
            "{label}: record size {record_size}, expected {LEGACY_LIST_U16_RECORD_SIZE}"
        );
    }

    let payload_start = header_end;
    let payload_len = usize::from(count)
        .checked_mul(usize::from(record_size))
        .context("legacy list payload overflow")?;
    let payload_end = payload_start
        .checked_add(payload_len)
        .context("legacy list payload end overflow")?;
    if payload_end > chunk_end {
        bail!("{label}: list payload exceeds bounded chunk");
    }

    let stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let ids = (0..usize::from(count))
        .map(|index| {
            let offset = payload_start + index * usize::from(record_size);
            (
                read_u16(contents, offset).expect("legacy list payload bounds checked"),
                RawSpan {
                    stream: stream.clone(),
                    offset: offset as u64,
                    len: 2,
                },
            )
        })
        .collect();

    Ok(LegacyIdList { ids })
}

fn legacy_text_shape_bounds(page: &Page, chunk: &[u8]) -> Option<RectEmu> {
    let xs = i64::from(read_i32(chunk, LEGACY_SHAPE_XS_OFFSET)?);
    let ys = i64::from(read_i32(chunk, LEGACY_SHAPE_YS_OFFSET)?);
    let xe = i64::from(read_i32(chunk, LEGACY_SHAPE_XE_OFFSET)?);
    let ye = i64::from(read_i32(chunk, LEGACY_SHAPE_YE_OFFSET)?);
    let width = xe.checked_sub(xs)?;
    let height = ye.checked_sub(ys)?;
    if width <= 0 || height <= 0 {
        return None;
    }
    let x = page.size.width.get().checked_div(2)?.checked_add(xs)?;
    let y = page.size.height.get().checked_div(2)?.checked_add(ys)?;
    Some(RectEmu::new(
        LengthEmu::new(x),
        LengthEmu::new(y),
        LengthEmu::new(width),
        LengthEmu::new(height),
    ))
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let raw = bytes.get(offset..offset.checked_add(4)?)?;
    Some(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_relative_geometry_uses_center_origin_contract() {
        let page_id = PageId::from_canonical(pub_model::CanonicalId::from_bytes([1; 16]));
        let page = Page {
            id: page_id,
            size: Size2D::new(
                LengthEmu::new(7_772_400),
                LengthEmu::new(10_058_400),
            ),
            bleed: None,
            margins: None,
            children: Vec::new(),
            extensions: Vec::new(),
        };
        let mut chunk = vec![0_u8; 0x5a];
        chunk[LEGACY_SHAPE_XS_OFFSET..LEGACY_SHAPE_XS_OFFSET + 4]
            .copy_from_slice(&1_123_950_i32.to_le_bytes());
        chunk[LEGACY_SHAPE_YS_OFFSET..LEGACY_SHAPE_YS_OFFSET + 4]
            .copy_from_slice(&400_050_i32.to_le_bytes());
        chunk[LEGACY_SHAPE_XE_OFFSET..LEGACY_SHAPE_XE_OFFSET + 4]
            .copy_from_slice(&2_571_750_i32.to_le_bytes());
        chunk[LEGACY_SHAPE_YE_OFFSET..LEGACY_SHAPE_YE_OFFSET + 4]
            .copy_from_slice(&1_104_900_i32.to_le_bytes());

        let bounds = legacy_text_shape_bounds(&page, &chunk).unwrap();
        assert_eq!(bounds.x.get(), 5_010_150);
        assert_eq!(bounds.y.get(), 5_429_250);
        assert_eq!(bounds.width.get(), 1_447_800);
        assert_eq!(bounds.height.get(), 704_850);
    }
}
