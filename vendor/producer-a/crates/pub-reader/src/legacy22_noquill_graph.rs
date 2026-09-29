use super::{
    CONTENTS_STREAM_PATH, PubBridgeDiagnostic, PubEffectivePageProjection,
    PubEffectivePageProjectionAuthority, PubExplicitShapePaintSource, PubNodePayload, PubSourceGraph,
    PubSourceGraphBuild, PubStoryFrameSource, ROLE_DOCUMENT, ROLE_NODE, ROLE_PAGE, ROLE_STORY,
    derive_pub_id, source_ref,
};
use anyhow::{Context, Result, bail};
use pub_contents::{
    Legacy0x22Directory, Legacy0x22DirectoryEntry, parse_legacy_0x22_directory,
    parse_legacy_0x22_formatting_descriptor, parse_legacy_0x22_text_info_map,
};
use pub_core::{RawSpan, StreamPath};
use pub_model::{
    Affine2D, AuthorityClass, Document, DocumentId, LengthEmu, Node, NodeHeader, NodeId, NodeKind,
    Page, PageId, ReadConfidence, RectEmu, Sha256Digest, Size2D, SourceDescriptor, SourceRole,
    Story, StoryId,
};
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

#[derive(Debug, Clone)]
struct LegacyIdList {
    ids: Vec<(u16, RawSpan)>,
}

pub fn build_legacy_0x22_noquill_source_graph<R: Read + Seek>(
    mut reader: R,
    source_hash: Sha256Digest,
) -> Result<PubSourceGraphBuild> {
    reader.seek(SeekFrom::Start(0))?;
    let mut pub_bytes = Vec::new();
    reader.read_to_end(&mut pub_bytes)?;
    let contents =
        pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), CONTENTS_STREAM_PATH)
            .with_context(|| format!("read {CONTENTS_STREAM_PATH}"))?;
    build_legacy_0x22_noquill_from_contents(source_hash, &contents)
}

pub fn build_legacy_0x22_noquill_from_contents(
    source_hash: Sha256Digest,
    contents: &[u8],
) -> Result<PubSourceGraphBuild> {
    let source = SourceDescriptor {
        format: "pub".into(),
        format_version: Some("0x22-noquill".into()),
        adapter_version: format!("pub-rs/{}", env!("CARGO_PKG_VERSION")),
        source_hash,
    };
    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let directory = parse_legacy_0x22_directory(contents_stream.clone(), contents)
        .context("parse legacy no-Quill 0x22 Contents directory")?;
    let descriptor =
        parse_legacy_0x22_formatting_descriptor(contents_stream.clone(), contents)
            .context("parse legacy no-Quill text descriptor")?;

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

    let text_start = usize::try_from(descriptor.text_start).context("legacy text start overflow")?;
    let text_end = usize::try_from(descriptor.text_end).context("legacy text end overflow")?;
    let text_bytes = contents
        .get(text_start..text_end)
        .context("legacy no-Quill text range exceeds Contents")?;
    let mut story_by_owner = BTreeMap::<u16, StoryId>::new();

    let text_info = parse_legacy_0x22_text_info_map(contents_stream.clone(), contents)
        .context("parse legacy no-Quill text owner map")?;
    if let Some(text_info) = text_info {
        let mut previous_end = 0usize;
        for owner in text_info.ends {
            let end = usize::try_from(owner.relative_end_offset)
                .context("legacy owner text end overflow")?
                .checked_add(1)
                .context("legacy owner text end overflow")?;
            if end < previous_end || end > text_bytes.len() {
                bail!(
                    "legacy owner {} text boundary {} is invalid for text length {}",
                    owner.owner_id,
                    end,
                    text_bytes.len()
                );
            }
            if end == previous_end {
                continue;
            }
            let story_id = derive_legacy_story_id(&source_hash, owner.owner_id)?;
            let text = decode_bounded_legacy_ascii(&text_bytes[previous_end..end])
                .with_context(|| format!("decode no-Quill owner {} text", owner.owner_id))?;
            let absolute_start = text_start + previous_end;
            graph.stories.insert(
                story_id,
                Story {
                    id: story_id,
                    text,
                    paragraphs: Vec::new(),
                    runs: Vec::new(),
                    fields: Vec::new(),
                    hyperlinks: Vec::new(),
                    source_refs: vec![
                        source_ref(
                            &graph.source,
                            &RawSpan {
                                stream: contents_stream.clone(),
                                offset: absolute_start as u64,
                                len: (end - previous_end) as u64,
                            },
                            Some(format!("contents/0x22/noquill-owner/{}", owner.owner_id)),
                            Some("legacy_text".into()),
                            SourceRole::Semantic,
                            AuthorityClass::Authoritative,
                            ReadConfidence::Exact,
                        ),
                        source_ref(
                            &graph.source,
                            &owner.owner_id_source,
                            Some(format!("contents/0x22/noquill-owner/{}", owner.owner_id)),
                            Some("owner_id".into()),
                            SourceRole::Relation,
                            AuthorityClass::Authoritative,
                            ReadConfidence::Exact,
                        ),
                    ],
                },
            );
            story_by_owner.insert(owner.owner_id, story_id);
            previous_end = end;
        }

        if previous_end < text_bytes.len() {
            materialize_unowned_text_story(
                &mut graph,
                &source_hash,
                &contents_stream,
                text_bytes,
                text_start,
                previous_end,
            )?;
        }
    } else if !text_bytes.is_empty() {
        materialize_unowned_text_story(
            &mut graph,
            &source_hash,
            &contents_stream,
            text_bytes,
            text_start,
            0,
        )?;
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
                    reason: "legacy_noquill_object_type_not_admitted_v1".into(),
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

            let node_id = derive_legacy_node_id(&source_hash, child_object_id)?;
            let story_id = story_by_owner.get(&child_object_id).copied();
            let source_refs = vec![
                source_ref(
                    &graph.source,
                    &child_entry.entry_source,
                    Some(format!("contents/0x22/object/{child_object_id}")),
                    Some("directory_entry".into()),
                    SourceRole::Relation,
                    AuthorityClass::Authoritative,
                    ReadConfidence::Exact,
                ),
                source_ref(
                    &graph.source,
                    &RawSpan {
                        stream: contents_stream.clone(),
                        offset: child_entry.chunk_source.offset + LEGACY_SHAPE_XS_OFFSET as u64,
                        len: 16,
                    },
                    Some(format!("contents/0x22/object/{child_object_id}")),
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
                        story_frame: story_id.map(|story_id| PubStoryFrameSource {
                            text_id: u32::from(child_object_id),
                            story_id: Some(story_id),
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

    Ok(PubSourceGraphBuild {
        graph,
        effective_pages: PubEffectivePageProjection {
            authority: PubEffectivePageProjectionAuthority::RawDocumentPageList,
            page_ids: document_pages.clone(),
            raw_page_count: document_pages.len(),
            observed_scenario_page_ids: Vec::new(),
            scenario_evidence_list_count: 0,
        },
        diagnostics,
        typography_runs: Vec::new(),
    })
}

fn materialize_unowned_text_story(
    graph: &mut PubSourceGraph,
    source_hash: &Sha256Digest,
    stream: &StreamPath,
    text_bytes: &[u8],
    absolute_text_start: usize,
    relative_start: usize,
) -> Result<()> {
    let remaining = &text_bytes[relative_start..];
    if remaining.is_empty() {
        return Ok(());
    }
    let story_id = derive_unowned_text_story_id(
        source_hash,
        absolute_text_start + relative_start,
        remaining.len(),
    )?;
    let text = decode_bounded_legacy_ascii(remaining)?;
    graph.stories.insert(
        story_id,
        Story {
            id: story_id,
            text,
            paragraphs: Vec::new(),
            runs: Vec::new(),
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: vec![source_ref(
                &graph.source,
                &RawSpan {
                    stream: stream.clone(),
                    offset: (absolute_text_start + relative_start) as u64,
                    len: remaining.len() as u64,
                },
                Some(format!(
                    "contents/0x22/noquill-text-range/{:#x}+{}",
                    absolute_text_start + relative_start,
                    remaining.len()
                )),
                Some("legacy_text".into()),
                SourceRole::Semantic,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            )],
        },
    );
    Ok(())
}

fn decode_bounded_legacy_ascii(bytes: &[u8]) -> Result<String> {
    if bytes.iter().any(|byte| *byte >= 0x80) {
        bail!("legacy no-Quill V1 text contains non-ASCII bytes; codepage authority is not admitted");
    }
    String::from_utf8(bytes.to_vec()).context("legacy no-Quill ASCII text")
}

fn derive_legacy_document_id(source_hash: &Sha256Digest, object_id: u16) -> Result<DocumentId> {
    Ok(DocumentId::from_canonical(derive_pub_id(
        source_hash,
        &format!("contents/0x22/object/{object_id}"),
        ROLE_DOCUMENT,
    )?))
}

fn derive_legacy_page_id(source_hash: &Sha256Digest, object_id: u16) -> Result<PageId> {
    Ok(PageId::from_canonical(derive_pub_id(
        source_hash,
        &format!("contents/0x22/object/{object_id}"),
        ROLE_PAGE,
    )?))
}

fn derive_legacy_node_id(source_hash: &Sha256Digest, object_id: u16) -> Result<NodeId> {
    Ok(NodeId::from_canonical(derive_pub_id(
        source_hash,
        &format!("contents/0x22/object/{object_id}"),
        ROLE_NODE,
    )?))
}

fn derive_legacy_story_id(source_hash: &Sha256Digest, owner_id: u16) -> Result<StoryId> {
    Ok(StoryId::from_canonical(derive_pub_id(
        source_hash,
        &format!("contents/0x22/noquill-owner/{owner_id}"),
        ROLE_STORY,
    )?))
}

fn derive_unowned_text_story_id(
    source_hash: &Sha256Digest,
    absolute_start: usize,
    len: usize,
) -> Result<StoryId> {
    Ok(StoryId::from_canonical(derive_pub_id(
        source_hash,
        &format!("contents/0x22/noquill-text-range/{absolute_start:#x}+{len}"),
        ROLE_STORY,
    )?))
}

fn unique_entry_by_type<'a>(
    directory: &'a Legacy0x22Directory,
    chunk_type: u16,
    label: &str,
) -> Result<&'a Legacy0x22DirectoryEntry> {
    let mut matches = directory.entries.iter().filter(|entry| entry.chunk_type == chunk_type);
    let first = matches.next().with_context(|| format!("missing legacy {label}"))?;
    if matches.next().is_some() {
        bail!("multiple legacy {label} objects");
    }
    Ok(first)
}

fn chunk_bytes<'a>(contents: &'a [u8], entry: &Legacy0x22DirectoryEntry) -> Result<&'a [u8]> {
    let start = usize::try_from(entry.chunk_source.offset).context("legacy chunk offset overflow")?;
    let len = usize::try_from(entry.chunk_source.len).context("legacy chunk length overflow")?;
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
    let chunk_start = usize::try_from(entry.chunk_source.offset).context("legacy chunk offset overflow")?;
    let chunk_len = usize::try_from(entry.chunk_source.len).context("legacy chunk length overflow")?;
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
        bail!("{label}: record size {record_size}, expected {LEGACY_LIST_U16_RECORD_SIZE}");
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
    Ok(LegacyIdList {
        ids: (0..usize::from(count))
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
            .collect(),
    })
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
    fn ascii_decoder_fails_closed_on_unproven_codepage() {
        assert_eq!(decode_bounded_legacy_ascii(b"OPEN HOUSE").unwrap(), "OPEN HOUSE");
        assert!(decode_bounded_legacy_ascii(&[0x80]).is_err());
    }
}
