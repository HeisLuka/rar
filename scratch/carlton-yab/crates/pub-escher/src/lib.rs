mod assets;

pub use assets::{
    AssetReadError, BStoreInventory, BStoreSlot, BlipKind, DelayedBlip, DelayedBlipInventory,
    OFFICE_ART_BLIP_DIB, OFFICE_ART_BLIP_EMF, OFFICE_ART_BLIP_JPEG, OFFICE_ART_BLIP_PICT,
    OFFICE_ART_BLIP_PNG, OFFICE_ART_BLIP_TIFF, OFFICE_ART_BLIP_WMF, OFFICE_ART_BSTORE_CONTAINER,
    OFFICE_ART_FBSE, inspect_bstore, inspect_delayed_blips, resolve_delayed_blip,
};

use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const OFFICE_ART_DGG_CONTAINER: u16 = 0xF000;
pub const OFFICE_ART_DG_CONTAINER: u16 = 0xF002;
pub const OFFICE_ART_SPGR_CONTAINER: u16 = 0xF003;
pub const OFFICE_ART_SP_CONTAINER: u16 = 0xF004;
pub const OFFICE_ART_FSPGR: u16 = 0xF009;
pub const OFFICE_ART_FSP: u16 = 0xF00A;
pub const OFFICE_ART_FOPT: u16 = 0xF00B;
pub const OFFICE_ART_CLIENT_TEXTBOX: u16 = 0xF00D;
pub const OFFICE_ART_CHILD_ANCHOR: u16 = 0xF00F;
pub const OFFICE_ART_CLIENT_ANCHOR: u16 = 0xF010;
pub const OFFICE_ART_CLIENT_DATA: u16 = 0xF011;
pub const OFFICE_ART_TERTIARY_FOPT: u16 = 0xF122;

/// MS-ODRAW picture crop properties. Values are preserved as raw OfficeArt
/// scalar payloads here; caller layers decide whether they can safely promote
/// them to a normalized content-transform semantic.
pub const OFFICE_ART_PROPERTY_CROP_FROM_TOP: u16 = 0x0100;
pub const OFFICE_ART_PROPERTY_CROP_FROM_BOTTOM: u16 = 0x0101;
pub const OFFICE_ART_PROPERTY_CROP_FROM_LEFT: u16 = 0x0102;
pub const OFFICE_ART_PROPERTY_CROP_FROM_RIGHT: u16 = 0x0103;

/// MS-ODRAW `pib`: one-based OfficeArtBStore entry identity when fBid=1.
pub const OFFICE_ART_PROPERTY_PIB: u16 = 0x0104;

pub const PUBLISHER_FIELD_XS: u16 = 0x2001;
pub const PUBLISHER_FIELD_YS: u16 = 0x2002;
pub const PUBLISHER_FIELD_XE: u16 = 0x2003;
pub const PUBLISHER_FIELD_YE: u16 = 0x2004;
pub const PUBLISHER_FIELD_SHAPE_ID: u16 = 0x6801;

pub const SP_CONTAINER_INVENTORY_SCHEMA_VERSION: u32 = 2;

/// Сырая запись OfficeArtFOPTE и её сложные данные, если они присутствуют.
///
/// Поле `op` сохраняется как исходное 32-битное значение. Его смысл зависит
/// от конкретного свойства. При установленном `fComplex` спецификация
/// MS-ODRAW определяет `op` как размер сложных данных в байтах, а не как
/// обычное скалярное значение свойства.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fopte {
    pub opid: u16,
    pub op: u32,
    pub source: RawSpan,
    pub complex_source: Option<RawSpan>,
    pub complex_data: Option<Vec<u8>>,
}

impl Fopte {
    pub fn property_id(&self) -> u16 {
        self.opid & 0x3fff
    }

    pub fn f_bid(&self) -> bool {
        self.opid & 0x4000 != 0
    }

    pub fn f_complex(&self) -> bool {
        self.opid & 0x8000 != 0
    }

    /// Возвращает true только когда бит fBid имеет смысл для данной записи.
    pub fn op_is_blip_id(&self) -> bool {
        self.f_bid() && !self.f_complex()
    }

    pub fn complex_size(&self) -> Option<u32> {
        self.f_complex().then_some(self.op)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficeArtHeader {
    pub rec_ver: u8,
    pub rec_instance: u16,
    pub rec_type: u16,
    pub rec_len: u32,
    pub source: RawSpan,
}

impl OfficeArtHeader {
    pub fn is_container(&self) -> bool {
        self.rec_ver == 0x0f
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FspRecord {
    pub spid: u32,
    pub flags: u32,
    /// Для OfficeArtFSP recInstance является MSOSPT / shape type.
    pub shape_type: u16,
    pub source: RawSpan,
    pub trailing_source: Option<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficeArtCoordinateRect {
    pub x_left: i32,
    pub y_top: i32,
    pub x_right: i32,
    pub y_bottom: i32,
    pub source: RawSpan,
    pub trailing_source: Option<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherField {
    pub id: u16,
    pub value: u32,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherFieldRecord {
    pub duplicated_length: u32,
    pub duplicated_length_source: RawSpan,
    pub fields: Vec<PublisherField>,
    pub trailing_source: Option<RawSpan>,
}

impl PublisherFieldRecord {
    pub fn values(&self, id: u16) -> impl Iterator<Item = u32> + '_ {
        self.fields
            .iter()
            .filter(move |field| field.id == id)
            .map(|field| field.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoptRecord {
    pub properties: Vec<Fopte>,
    pub trailing_source: Option<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OfficeArtBody {
    Container { children: Vec<OfficeArtRecord> },
    Fspgr(OfficeArtCoordinateRect),
    Fsp(FspRecord),
    Fopt(FoptRecord),
    ChildAnchor(OfficeArtCoordinateRect),
    PublisherFields(PublisherFieldRecord),
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficeArtRecord {
    pub header: OfficeArtHeader,
    /// Только физически объявленная запись: 8-byte header + recLen payload.
    pub source: RawSpan,
    pub payload_source: RawSpan,
    /// libmspub пропускает 4 байта после DGG/DG при переходе к следующему
    /// sibling. На реальном Sample.pub последний DG заканчивается ровно на
    /// границе stream, поэтому эти байты являются optional sibling padding,
    /// а не обязательной частью самой OfficeArt-записи.
    pub sibling_tail_source: Option<RawSpan>,
    pub body: OfficeArtBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficeArtStream {
    pub source: RawSpan,
    pub records: Vec<OfficeArtRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoptObservation {
    pub rec_type: u16,
    pub source: RawSpan,
    pub properties: Vec<Fopte>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpContainerObservation {
    pub source: RawSpan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group_shape_source: Option<RawSpan>,
    pub fspgr: Option<OfficeArtCoordinateRect>,
    pub fsp: Option<FspRecord>,
    pub fopts: Vec<FoptObservation>,
    pub client_anchor: Option<PublisherFieldRecord>,
    pub client_data: Option<PublisherFieldRecord>,
    pub client_textbox: Option<RawSpan>,
    pub child_anchor: Option<OfficeArtCoordinateRect>,
    pub unknown_children: Vec<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpContainerInventory {
    pub schema_version: u32,
    pub stream: StreamPath,
    pub stream_len: u64,
    pub shapes: Vec<SpContainerObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfficeArtReadError {
    TruncatedHeader {
        offset: usize,
        available: usize,
    },
    DeclaredRangeOutOfBounds {
        offset: usize,
        rec_type: u16,
        declared_len: u32,
        limit: usize,
    },
    FspTooShort {
        offset: usize,
        len: usize,
    },
    CoordinateRectTooShort {
        offset: usize,
        rec_type: u16,
        len: usize,
    },
    FoptTableTooShort {
        offset: usize,
        count: u16,
        len: usize,
    },
    ComplexPropertyOutOfBounds {
        offset: usize,
        opid: u16,
        requested: u32,
        available: usize,
    },
    PublisherFieldRecordTooShort {
        offset: usize,
        rec_type: u16,
        len: usize,
    },
    OffsetTooLarge {
        offset: usize,
    },
}

impl fmt::Display for OfficeArtReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { offset, available } => write!(
                f,
                "обрезанный OfficeArt header по смещению {offset}: доступно {available} байт"
            ),
            Self::DeclaredRangeOutOfBounds {
                offset,
                rec_type,
                declared_len,
                limit,
            } => write!(
                f,
                "OfficeArt record 0x{rec_type:04X} по смещению {offset} объявляет payload {declared_len} байт за границей {limit}"
            ),
            Self::FspTooShort { offset, len } => {
                write!(f, "OfficeArtFSP по смещению {offset} короче 8 байт: {len}")
            }
            Self::CoordinateRectTooShort {
                offset,
                rec_type,
                len,
            } => write!(
                f,
                "OfficeArt coordinate rect 0x{rec_type:04X} по смещению {offset} короче 16 байт: {len}"
            ),
            Self::FoptTableTooShort { offset, count, len } => write!(
                f,
                "OfficeArtFOPT по смещению {offset}: recInstance={count}, но payload {len} байт не вмещает таблицу FOPTE"
            ),
            Self::ComplexPropertyOutOfBounds {
                offset,
                opid,
                requested,
                available,
            } => write!(
                f,
                "complex FOPTE 0x{opid:04X} по смещению {offset} требует {requested} байт, доступно {available}"
            ),
            Self::PublisherFieldRecordTooShort {
                offset,
                rec_type,
                len,
            } => write!(
                f,
                "Publisher OfficeArt record 0x{rec_type:04X} по смещению {offset} короче 4-байтного duplicated-length prefix: {len}"
            ),
            Self::OffsetTooLarge { offset } => {
                write!(f, "OfficeArt offset {offset} не помещается в RawSpan")
            }
        }
    }
}

impl std::error::Error for OfficeArtReadError {}

pub fn parse_officeart_stream(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<OfficeArtStream, OfficeArtReadError> {
    let records = parse_sequence(bytes, &stream, 0, bytes.len())?;
    Ok(OfficeArtStream {
        source: raw_span(&stream, 0, bytes.len())?,
        records,
    })
}

pub fn inspect_sp_containers(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<SpContainerInventory, OfficeArtReadError> {
    let parsed = parse_officeart_stream(stream.clone(), bytes)?;
    let mut shapes = Vec::new();
    collect_sp_containers(&parsed.records, &mut shapes);

    Ok(SpContainerInventory {
        schema_version: SP_CONTAINER_INVENTORY_SCHEMA_VERSION,
        stream,
        stream_len: bytes.len() as u64,
        shapes,
    })
}

fn parse_sequence(
    bytes: &[u8],
    stream: &StreamPath,
    start: usize,
    limit: usize,
) -> Result<Vec<OfficeArtRecord>, OfficeArtReadError> {
    let mut records = Vec::new();
    let mut offset = start;

    while offset < limit {
        let record = parse_record_at(bytes, stream, offset, limit)?;
        let physical_end = record
            .sibling_tail_source
            .as_ref()
            .unwrap_or(&record.source)
            .end()
            .ok_or(OfficeArtReadError::OffsetTooLarge { offset })?;
        let next = usize::try_from(physical_end)
            .map_err(|_| OfficeArtReadError::OffsetTooLarge { offset })?;
        if next <= offset {
            return Err(OfficeArtReadError::OffsetTooLarge { offset });
        }
        records.push(record);
        offset = next;
    }

    Ok(records)
}

fn parse_record_at(
    bytes: &[u8],
    stream: &StreamPath,
    offset: usize,
    limit: usize,
) -> Result<OfficeArtRecord, OfficeArtReadError> {
    let available = limit.saturating_sub(offset);
    if available < 8 {
        return Err(OfficeArtReadError::TruncatedHeader { offset, available });
    }

    let initial = read_u16(bytes, offset);
    let rec_ver = (initial & 0x000f) as u8;
    let rec_instance = initial >> 4;
    let rec_type = read_u16(bytes, offset + 2);
    let rec_len = read_u32(bytes, offset + 4);

    let payload_start = offset + 8;
    let payload_len =
        usize::try_from(rec_len).map_err(|_| OfficeArtReadError::DeclaredRangeOutOfBounds {
            offset,
            rec_type,
            declared_len: rec_len,
            limit,
        })?;
    let payload_end = payload_start.checked_add(payload_len).ok_or(
        OfficeArtReadError::DeclaredRangeOutOfBounds {
            offset,
            rec_type,
            declared_len: rec_len,
            limit,
        },
    )?;
    if payload_end > limit || payload_end > bytes.len() {
        return Err(OfficeArtReadError::DeclaredRangeOutOfBounds {
            offset,
            rec_type,
            declared_len: rec_len,
            limit,
        });
    }

    let sibling_tail_len = publisher_sibling_tail_len(rec_type);
    let sibling_tail_source = if sibling_tail_len == 0 {
        None
    } else {
        payload_end
            .checked_add(sibling_tail_len)
            .filter(|tail_end| *tail_end <= limit && *tail_end <= bytes.len())
            .map(|_| raw_span(stream, payload_end, sibling_tail_len))
            .transpose()?
    };

    let header = OfficeArtHeader {
        rec_ver,
        rec_instance,
        rec_type,
        rec_len,
        source: raw_span(stream, offset, 8)?,
    };
    let payload_source = raw_span(stream, payload_start, payload_len)?;

    let body = if header.is_container() {
        OfficeArtBody::Container {
            children: parse_sequence(bytes, stream, payload_start, payload_end)?,
        }
    } else {
        match rec_type {
            OFFICE_ART_FSPGR => OfficeArtBody::Fspgr(parse_coordinate_rect(
                bytes,
                stream,
                payload_start,
                payload_end,
                rec_type,
            )?),
            OFFICE_ART_FSP => OfficeArtBody::Fsp(parse_fsp(
                bytes,
                stream,
                payload_start,
                payload_end,
                rec_instance,
            )?),
            OFFICE_ART_FOPT | OFFICE_ART_TERTIARY_FOPT => OfficeArtBody::Fopt(parse_fopt(
                bytes,
                stream,
                payload_start,
                payload_end,
                rec_instance,
            )?),
            OFFICE_ART_CHILD_ANCHOR => OfficeArtBody::ChildAnchor(parse_coordinate_rect(
                bytes,
                stream,
                payload_start,
                payload_end,
                rec_type,
            )?),
            OFFICE_ART_CLIENT_ANCHOR | OFFICE_ART_CLIENT_DATA => OfficeArtBody::PublisherFields(
                parse_publisher_fields(bytes, stream, payload_start, payload_end, rec_type)?,
            ),
            _ => OfficeArtBody::Raw,
        }
    };

    Ok(OfficeArtRecord {
        header,
        source: raw_span(stream, offset, payload_end - offset)?,
        payload_source,
        sibling_tail_source,
        body,
    })
}

fn parse_coordinate_rect(
    bytes: &[u8],
    stream: &StreamPath,
    start: usize,
    end: usize,
    rec_type: u16,
) -> Result<OfficeArtCoordinateRect, OfficeArtReadError> {
    let len = end - start;
    if len < 16 {
        return Err(OfficeArtReadError::CoordinateRectTooShort {
            offset: start,
            rec_type,
            len,
        });
    }

    Ok(OfficeArtCoordinateRect {
        x_left: read_i32(bytes, start),
        y_top: read_i32(bytes, start + 4),
        x_right: read_i32(bytes, start + 8),
        y_bottom: read_i32(bytes, start + 12),
        source: raw_span(stream, start, 16)?,
        trailing_source: (len > 16)
            .then(|| raw_span(stream, start + 16, len - 16))
            .transpose()?,
    })
}

fn parse_fsp(
    bytes: &[u8],
    stream: &StreamPath,
    start: usize,
    end: usize,
    shape_type: u16,
) -> Result<FspRecord, OfficeArtReadError> {
    let len = end - start;
    if len < 8 {
        return Err(OfficeArtReadError::FspTooShort { offset: start, len });
    }

    Ok(FspRecord {
        spid: read_u32(bytes, start),
        flags: read_u32(bytes, start + 4),
        shape_type,
        source: raw_span(stream, start, 8)?,
        trailing_source: (len > 8)
            .then(|| raw_span(stream, start + 8, len - 8))
            .transpose()?,
    })
}

fn parse_fopt(
    bytes: &[u8],
    stream: &StreamPath,
    start: usize,
    end: usize,
    count: u16,
) -> Result<FoptRecord, OfficeArtReadError> {
    let payload_len = end - start;
    let table_len =
        usize::from(count)
            .checked_mul(6)
            .ok_or(OfficeArtReadError::FoptTableTooShort {
                offset: start,
                count,
                len: payload_len,
            })?;
    if table_len > payload_len {
        return Err(OfficeArtReadError::FoptTableTooShort {
            offset: start,
            count,
            len: payload_len,
        });
    }

    let mut properties = Vec::with_capacity(usize::from(count));
    for index in 0..usize::from(count) {
        let offset = start + index * 6;
        properties.push(Fopte {
            opid: read_u16(bytes, offset),
            op: read_u32(bytes, offset + 2),
            source: raw_span(stream, offset, 6)?,
            complex_source: None,
            complex_data: None,
        });
    }

    let mut complex_offset = start + table_len;
    for property in &mut properties {
        if !property.f_complex() {
            continue;
        }

        let requested = property.op;
        let requested_usize = usize::try_from(requested).map_err(|_| {
            OfficeArtReadError::ComplexPropertyOutOfBounds {
                offset: complex_offset,
                opid: property.opid,
                requested,
                available: end.saturating_sub(complex_offset),
            }
        })?;
        let complex_end = complex_offset.checked_add(requested_usize).ok_or(
            OfficeArtReadError::ComplexPropertyOutOfBounds {
                offset: complex_offset,
                opid: property.opid,
                requested,
                available: end.saturating_sub(complex_offset),
            },
        )?;
        if complex_end > end {
            return Err(OfficeArtReadError::ComplexPropertyOutOfBounds {
                offset: complex_offset,
                opid: property.opid,
                requested,
                available: end.saturating_sub(complex_offset),
            });
        }

        property.complex_source = Some(raw_span(stream, complex_offset, requested_usize)?);
        property.complex_data = Some(bytes[complex_offset..complex_end].to_vec());
        complex_offset = complex_end;
    }

    Ok(FoptRecord {
        properties,
        trailing_source: (complex_offset < end)
            .then(|| raw_span(stream, complex_offset, end - complex_offset))
            .transpose()?,
    })
}

fn parse_publisher_fields(
    bytes: &[u8],
    stream: &StreamPath,
    start: usize,
    end: usize,
    rec_type: u16,
) -> Result<PublisherFieldRecord, OfficeArtReadError> {
    let len = end - start;
    if len < 4 {
        return Err(OfficeArtReadError::PublisherFieldRecordTooShort {
            offset: start,
            rec_type,
            len,
        });
    }

    let duplicated_length = read_u32(bytes, start);
    let mut offset = start + 4;
    let mut fields = Vec::new();

    while end.saturating_sub(offset) >= 6 {
        fields.push(PublisherField {
            id: read_u16(bytes, offset),
            value: read_u32(bytes, offset + 2),
            source: raw_span(stream, offset, 6)?,
        });
        offset += 6;
    }

    Ok(PublisherFieldRecord {
        duplicated_length,
        duplicated_length_source: raw_span(stream, start, 4)?,
        fields,
        trailing_source: (offset < end)
            .then(|| raw_span(stream, offset, end - offset))
            .transpose()?,
    })
}

fn collect_sp_containers(records: &[OfficeArtRecord], output: &mut Vec<SpContainerObservation>) {
    collect_sp_containers_in_scope(records, None, output);
}

fn collect_sp_containers_in_scope(
    records: &[OfficeArtRecord],
    parent_group_shape_source: Option<&RawSpan>,
    output: &mut Vec<SpContainerObservation>,
) {
    for record in records {
        if record.header.rec_type == OFFICE_ART_SPGR_CONTAINER {
            if let OfficeArtBody::Container { children } = &record.body {
                collect_spgr_container(children, parent_group_shape_source, output);
            }
            continue;
        }

        if record.header.rec_type == OFFICE_ART_SP_CONTAINER {
            if let OfficeArtBody::Container { children } = &record.body {
                output.push(observe_sp_container(
                    record,
                    children,
                    parent_group_shape_source.cloned(),
                ));
            }
            continue;
        }

        if let OfficeArtBody::Container { children } = &record.body {
            collect_sp_containers_in_scope(children, parent_group_shape_source, output);
        }
    }
}

fn collect_spgr_container(
    children: &[OfficeArtRecord],
    inherited_parent_group_shape_source: Option<&RawSpan>,
    output: &mut Vec<SpContainerObservation>,
) {
    let group_shape_source = children
        .iter()
        .find(|child| child.header.rec_type == OFFICE_ART_SP_CONTAINER)
        .map(|child| child.source.clone());

    let mut saw_group_shape = false;
    for child in children {
        match (&child.body, child.header.rec_type) {
            (
                OfficeArtBody::Container {
                    children: shape_children,
                },
                OFFICE_ART_SP_CONTAINER,
            ) => {
                let parent = if !saw_group_shape {
                    saw_group_shape = true;
                    inherited_parent_group_shape_source.cloned()
                } else {
                    group_shape_source.clone()
                };
                output.push(observe_sp_container(child, shape_children, parent));
            }
            (OfficeArtBody::Container { children: nested }, OFFICE_ART_SPGR_CONTAINER) => {
                collect_spgr_container(
                    nested,
                    group_shape_source
                        .as_ref()
                        .or(inherited_parent_group_shape_source),
                    output,
                );
            }
            (OfficeArtBody::Container { children: nested }, _) => {
                collect_sp_containers_in_scope(
                    nested,
                    group_shape_source
                        .as_ref()
                        .or(inherited_parent_group_shape_source),
                    output,
                );
            }
            _ => {}
        }
    }
}

fn observe_sp_container(
    container: &OfficeArtRecord,
    children: &[OfficeArtRecord],
    parent_group_shape_source: Option<RawSpan>,
) -> SpContainerObservation {
    let mut fspgr = None;
    let mut fsp = None;
    let mut fopts = Vec::new();
    let mut client_anchor = None;
    let mut client_data = None;
    let mut client_textbox = None;
    let mut child_anchor = None;
    let mut unknown_children = Vec::new();

    for child in children {
        match (&child.body, child.header.rec_type) {
            (OfficeArtBody::Fspgr(record), OFFICE_ART_FSPGR) => fspgr = Some(record.clone()),
            (OfficeArtBody::Fsp(record), OFFICE_ART_FSP) => fsp = Some(record.clone()),
            (OfficeArtBody::Fopt(record), OFFICE_ART_FOPT | OFFICE_ART_TERTIARY_FOPT) => {
                fopts.push(FoptObservation {
                    rec_type: child.header.rec_type,
                    source: child.source.clone(),
                    properties: record.properties.clone(),
                });
            }
            (OfficeArtBody::PublisherFields(record), OFFICE_ART_CLIENT_ANCHOR) => {
                client_anchor = Some(record.clone());
            }
            (OfficeArtBody::PublisherFields(record), OFFICE_ART_CLIENT_DATA) => {
                client_data = Some(record.clone());
            }
            (_, OFFICE_ART_CLIENT_TEXTBOX) => {
                client_textbox = Some(child.payload_source.clone());
            }
            (OfficeArtBody::ChildAnchor(record), OFFICE_ART_CHILD_ANCHOR) => {
                child_anchor = Some(record.clone());
            }
            _ => unknown_children.push(child.source.clone()),
        }
    }

    SpContainerObservation {
        source: container.source.clone(),
        parent_group_shape_source,
        fspgr,
        fsp,
        fopts,
        client_anchor,
        client_data,
        client_textbox,
        child_anchor,
        unknown_children,
    }
}

fn publisher_sibling_tail_len(rec_type: u16) -> usize {
    match rec_type {
        OFFICE_ART_DGG_CONTAINER | OFFICE_ART_DG_CONTAINER => 4,
        _ => 0,
    }
}

fn raw_span(stream: &StreamPath, offset: usize, len: usize) -> Result<RawSpan, OfficeArtReadError> {
    Ok(RawSpan {
        stream: stream.clone(),
        offset: u64::try_from(offset).map_err(|_| OfficeArtReadError::OffsetTooLarge { offset })?,
        len: u64::try_from(len).map_err(|_| OfficeArtReadError::OffsetTooLarge { offset })?,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> StreamPath {
        StreamPath("/Escher/EscherStm".into())
    }

    fn header(initial: u16, rec_type: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&initial.to_le_bytes());
        bytes.extend_from_slice(&rec_type.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn container(rec_type: u16, children: &[u8]) -> Vec<u8> {
        header(0x000f, rec_type, children)
    }

    #[test]
    fn opid_flags_are_kept_separate_from_property_id() {
        let entry = Fopte {
            opid: 0xc104,
            op: 12,
            source: RawSpan {
                stream: stream(),
                offset: 0,
                len: 6,
            },
            complex_source: None,
            complex_data: Some(vec![0; 12]),
        };

        assert_eq!(entry.property_id(), 0x0104);
        assert!(entry.f_bid());
        assert!(entry.f_complex());
        assert!(!entry.op_is_blip_id());
        assert_eq!(entry.complex_size(), Some(12));
    }

    #[test]
    fn non_complex_fbid_can_identify_blip_reference() {
        let entry = Fopte {
            opid: 0x4104,
            op: 3,
            source: RawSpan {
                stream: stream(),
                offset: 0,
                len: 6,
            },
            complex_source: None,
            complex_data: None,
        };

        assert_eq!(entry.property_id(), 0x0104);
        assert!(entry.f_bid());
        assert!(!entry.f_complex());
        assert!(entry.op_is_blip_id());
        assert_eq!(entry.complex_size(), None);
    }

    #[test]
    fn parses_fspgr_and_child_anchor_as_signed_coordinate_rects() {
        let mut group_payload = Vec::new();
        for value in [-100_i32, -200, 300, 400] {
            group_payload.extend_from_slice(&value.to_le_bytes());
        }
        let group_bytes = header(0x0001, OFFICE_ART_FSPGR, &group_payload);
        let parsed = parse_officeart_stream(stream(), &group_bytes).expect("FSPGR parses");
        let OfficeArtBody::Fspgr(group) = &parsed.records[0].body else {
            panic!("expected FSPGR");
        };
        assert_eq!(
            (group.x_left, group.y_top, group.x_right, group.y_bottom),
            (-100, -200, 300, 400)
        );
        assert_eq!(group.source.len, 16);
        assert!(group.trailing_source.is_none());

        let mut child_payload = Vec::new();
        for value in [-10_i32, 20, 30, -40] {
            child_payload.extend_from_slice(&value.to_le_bytes());
        }
        let child_bytes = header(0x0000, OFFICE_ART_CHILD_ANCHOR, &child_payload);
        let parsed = parse_officeart_stream(stream(), &child_bytes).expect("ChildAnchor parses");
        let OfficeArtBody::ChildAnchor(child) = &parsed.records[0].body else {
            panic!("expected ChildAnchor");
        };
        assert_eq!(
            (child.x_left, child.y_top, child.x_right, child.y_bottom),
            (-10, 20, 30, -40)
        );
    }

    #[test]
    fn sp_inventory_preserves_group_shape_parentage() {
        let mut fspgr_payload = Vec::new();
        for value in [0_i32, 0, 1000, 1000] {
            fspgr_payload.extend_from_slice(&value.to_le_bytes());
        }
        let fspgr = header(0x0001, OFFICE_ART_FSPGR, &fspgr_payload);

        let mut group_fsp_payload = Vec::new();
        group_fsp_payload.extend_from_slice(&100_u32.to_le_bytes());
        group_fsp_payload.extend_from_slice(&1_u32.to_le_bytes());
        let group_fsp = header(0x0002, OFFICE_ART_FSP, &group_fsp_payload);

        let mut group_children = Vec::new();
        group_children.extend_from_slice(&fspgr);
        group_children.extend_from_slice(&group_fsp);
        let group_shape = container(OFFICE_ART_SP_CONTAINER, &group_children);

        let mut child_fsp_payload = Vec::new();
        child_fsp_payload.extend_from_slice(&101_u32.to_le_bytes());
        child_fsp_payload.extend_from_slice(&2_u32.to_le_bytes());
        let child_fsp = header(0x0002, OFFICE_ART_FSP, &child_fsp_payload);

        let mut child_anchor_payload = Vec::new();
        for value in [100_i32, 200, 300, 400] {
            child_anchor_payload.extend_from_slice(&value.to_le_bytes());
        }
        let child_anchor = header(0x0000, OFFICE_ART_CHILD_ANCHOR, &child_anchor_payload);

        let mut child_children = Vec::new();
        child_children.extend_from_slice(&child_fsp);
        child_children.extend_from_slice(&child_anchor);
        let child_shape = container(OFFICE_ART_SP_CONTAINER, &child_children);

        let mut group_container_children = Vec::new();
        group_container_children.extend_from_slice(&group_shape);
        group_container_children.extend_from_slice(&child_shape);
        let bytes = container(OFFICE_ART_SPGR_CONTAINER, &group_container_children);

        let inventory = inspect_sp_containers(stream(), &bytes).expect("group inventory");
        assert_eq!(inventory.schema_version, 2);
        assert_eq!(inventory.shapes.len(), 2);

        let group = &inventory.shapes[0];
        let child = &inventory.shapes[1];
        assert!(group.parent_group_shape_source.is_none());
        assert!(group.fspgr.is_some());
        assert!(group.child_anchor.is_none());

        assert_eq!(
            child.parent_group_shape_source.as_ref(),
            Some(&group.source)
        );
        assert!(child.fspgr.is_none());
        assert_eq!(
            child.child_anchor.as_ref().map(|anchor| (
                anchor.x_left,
                anchor.y_top,
                anchor.x_right,
                anchor.y_bottom
            )),
            Some((100, 200, 300, 400))
        );
    }

    #[test]
    fn parses_fsp_and_shape_type_from_record_instance() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1082u32.to_le_bytes());
        payload.extend_from_slice(&0x00000A00u32.to_le_bytes());

        let bytes = header((1 << 4) | 0x2, OFFICE_ART_FSP, &payload);
        let parsed = parse_officeart_stream(stream(), &bytes).expect("FSP должен читаться");

        let OfficeArtBody::Fsp(fsp) = &parsed.records[0].body else {
            panic!("ожидался FSP");
        };
        assert_eq!(fsp.shape_type, 1);
        assert_eq!(fsp.spid, 1082);
        assert_eq!(fsp.flags, 0x00000A00);
    }

    #[test]
    fn parses_fopt_scalar_and_complex_payload_in_order() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x0101u16.to_le_bytes());
        payload.extend_from_slice(&6168u32.to_le_bytes());
        payload.extend_from_slice(&0xC145u16.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&[1, 2, 3, 4]);

        let bytes = header((2 << 4) | 0x3, OFFICE_ART_FOPT, &payload);
        let parsed = parse_officeart_stream(stream(), &bytes).expect("FOPT должен читаться");

        let OfficeArtBody::Fopt(fopt) = &parsed.records[0].body else {
            panic!("ожидался FOPT");
        };
        assert_eq!(fopt.properties.len(), 2);
        assert_eq!(fopt.properties[0].property_id(), 0x0101);
        assert_eq!(fopt.properties[0].op, 6168);
        assert_eq!(fopt.properties[1].property_id(), 0x0145);
        assert_eq!(
            fopt.properties[1].complex_data.as_deref(),
            Some(&[1, 2, 3, 4][..])
        );
        assert_eq!(
            fopt.properties[1]
                .complex_source
                .as_ref()
                .map(|span| (span.offset, span.len)),
            Some((20, 4))
        );
        assert!(fopt.trailing_source.is_none());
    }

    #[test]
    fn rejects_complex_payload_that_crosses_record_boundary() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0xC145u16.to_le_bytes());
        payload.extend_from_slice(&5u32.to_le_bytes());
        payload.extend_from_slice(&[1, 2]);

        let bytes = header((1 << 4) | 0x3, OFFICE_ART_FOPT, &payload);
        assert!(matches!(
            parse_officeart_stream(stream(), &bytes),
            Err(OfficeArtReadError::ComplexPropertyOutOfBounds { .. })
        ));
    }

    #[test]
    fn parses_publisher_client_data_after_duplicated_length_prefix() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&10u32.to_le_bytes());
        payload.extend_from_slice(&PUBLISHER_FIELD_SHAPE_ID.to_le_bytes());
        payload.extend_from_slice(&295u32.to_le_bytes());

        let bytes = header(0x0018, OFFICE_ART_CLIENT_DATA, &payload);
        let parsed = parse_officeart_stream(stream(), &bytes).expect("ClientData должен читаться");

        let OfficeArtBody::PublisherFields(fields) = &parsed.records[0].body else {
            panic!("ожидался PublisherFields");
        };
        assert_eq!(fields.duplicated_length, 10);
        assert_eq!(
            fields.values(PUBLISHER_FIELD_SHAPE_ID).collect::<Vec<_>>(),
            vec![295]
        );
    }

    #[test]
    fn sp_container_inventory_keeps_textbox_as_raw_carrier() {
        let mut fsp_payload = Vec::new();
        fsp_payload.extend_from_slice(&1082u32.to_le_bytes());
        fsp_payload.extend_from_slice(&0u32.to_le_bytes());
        let fsp = header((1 << 4) | 0x2, OFFICE_ART_FSP, &fsp_payload);

        let textbox_payload = [0x0A, 0, 0, 0, 1, 0x20, 33, 0, 0, 0];
        let textbox = header(0x0018, OFFICE_ART_CLIENT_TEXTBOX, &textbox_payload);

        let mut children = Vec::new();
        children.extend_from_slice(&fsp);
        children.extend_from_slice(&textbox);
        let bytes = container(OFFICE_ART_SP_CONTAINER, &children);

        let inventory =
            inspect_sp_containers(stream(), &bytes).expect("SpContainer должен читаться");
        assert_eq!(inventory.shapes.len(), 1);
        let shape = &inventory.shapes[0];
        assert_eq!(shape.fsp.as_ref().map(|value| value.spid), Some(1082));
        assert_eq!(shape.client_textbox.as_ref().map(|span| span.len), Some(10));
    }

    #[test]
    fn publisher_dg_tail_is_optional_at_parent_boundary() {
        let child = header(0x0002, OFFICE_ART_FSP, &[0; 8]);
        let bytes = container(OFFICE_ART_DG_CONTAINER, &child);

        let parsed =
            parse_officeart_stream(stream(), &bytes).expect("DG на границе должен читаться");
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].source.len, bytes.len() as u64);
        assert!(parsed.records[0].sibling_tail_source.is_none());
    }

    #[test]
    fn publisher_dg_container_consumes_confirmed_four_byte_sibling_tail() {
        let child = header(0x0002, OFFICE_ART_FSP, &[0; 8]);
        let mut bytes = container(OFFICE_ART_DG_CONTAINER, &child);
        bytes.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

        let parsed = parse_officeart_stream(stream(), &bytes).expect("DG должен читаться");
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(
            parsed.records[0]
                .sibling_tail_source
                .as_ref()
                .map(|span| span.len),
            Some(4)
        );
    }

    #[test]
    fn record_cannot_read_past_parent_container() {
        let child = header(0x0002, OFFICE_ART_FSP, &[0; 8]);
        let mut broken = container(OFFICE_ART_SP_CONTAINER, &child);
        broken.truncate(broken.len() - 2);

        assert!(matches!(
            parse_officeart_stream(stream(), &broken),
            Err(OfficeArtReadError::DeclaredRangeOutOfBounds { .. })
        ));
    }
}
