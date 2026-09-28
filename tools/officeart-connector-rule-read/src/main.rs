use pub_core::{RawSpan, StreamPath};
use pub_escher::{OfficeArtBody, OfficeArtRecord, parse_officeart_stream};
use serde::Serialize;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;

const OFFICE_ART_SOLVER_CONTAINER: u16 = 0xF005;
const OFFICE_ART_FSP: u16 = 0xF00A;
const OFFICE_ART_SP_CONTAINER: u16 = 0xF004;
const OFFICE_ART_CONNECTOR_RULE: u16 = 0xF012;
const FSP_CONNECTOR_BIT: u32 = 1 << 8;
const CONNECTOR_RULE_PAYLOAD_LEN: u32 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ConnectorRuleObservationV1 {
    record_source: RawSpan,
    payload_source: RawSpan,
    ruid: u32,
    spid_a: u32,
    spid_b: u32,
    spid_c: u32,
    cpti_a: u32,
    cpti_b: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SolverContainerObservationV1 {
    source: RawSpan,
    connector_rules: Vec<ConnectorRuleObservationV1>,
    unknown_children: Vec<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ConnectorShapeObservationV1 {
    sp_container_source: RawSpan,
    fsp_source: RawSpan,
    spid: u32,
    raw_flags: u32,
    f_connector: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ConnectorRuleInventoryV1 {
    schema_version: u32,
    logical_stream: StreamPath,
    stream_len: u64,
    solver_containers: Vec<SolverContainerObservationV1>,
    connector_shapes: Vec<ConnectorShapeObservationV1>,
}

#[derive(Debug)]
enum ConnectorReadError {
    OfficeArt(pub_escher::OfficeArtReadError),
    ConnectorRuleHeader {
        offset: u64,
        rec_ver: u8,
        rec_instance: u16,
        rec_len: u32,
    },
    SpanOutOfBounds {
        offset: u64,
        len: u64,
        stream_len: usize,
    },
}

impl fmt::Display for ConnectorReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OfficeArt(error) => write!(f, "{error}"),
            Self::ConnectorRuleHeader {
                offset,
                rec_ver,
                rec_instance,
                rec_len,
            } => write!(
                f,
                "OfficeArtFConnectorRule at {offset} must be recVer=1, recInstance=0, recLen=24; got recVer={rec_ver}, recInstance={rec_instance}, recLen={rec_len}"
            ),
            Self::SpanOutOfBounds {
                offset,
                len,
                stream_len,
            } => write!(
                f,
                "OfficeArt payload span {offset}+{len} is outside stream length {stream_len}"
            ),
        }
    }
}

impl Error for ConnectorReadError {}

impl From<pub_escher::OfficeArtReadError> for ConnectorReadError {
    fn from(value: pub_escher::OfficeArtReadError) -> Self {
        Self::OfficeArt(value)
    }
}

fn span_slice<'a>(bytes: &'a [u8], span: &RawSpan) -> Result<&'a [u8], ConnectorReadError> {
    let start = usize::try_from(span.offset).map_err(|_| ConnectorReadError::SpanOutOfBounds {
        offset: span.offset,
        len: span.len,
        stream_len: bytes.len(),
    })?;
    let len = usize::try_from(span.len).map_err(|_| ConnectorReadError::SpanOutOfBounds {
        offset: span.offset,
        len: span.len,
        stream_len: bytes.len(),
    })?;
    let end = start.checked_add(len).ok_or(ConnectorReadError::SpanOutOfBounds {
        offset: span.offset,
        len: span.len,
        stream_len: bytes.len(),
    })?;
    bytes.get(start..end).ok_or(ConnectorReadError::SpanOutOfBounds {
        offset: span.offset,
        len: span.len,
        stream_len: bytes.len(),
    })
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bounded connector payload"))
}

fn decode_connector_rule(
    record: &OfficeArtRecord,
    bytes: &[u8],
) -> Result<ConnectorRuleObservationV1, ConnectorReadError> {
    if record.header.rec_ver != 1
        || record.header.rec_instance != 0
        || record.header.rec_len != CONNECTOR_RULE_PAYLOAD_LEN
    {
        return Err(ConnectorReadError::ConnectorRuleHeader {
            offset: record.source.offset,
            rec_ver: record.header.rec_ver,
            rec_instance: record.header.rec_instance,
            rec_len: record.header.rec_len,
        });
    }

    let payload = span_slice(bytes, &record.payload_source)?;
    if payload.len() != CONNECTOR_RULE_PAYLOAD_LEN as usize {
        return Err(ConnectorReadError::ConnectorRuleHeader {
            offset: record.source.offset,
            rec_ver: record.header.rec_ver,
            rec_instance: record.header.rec_instance,
            rec_len: record.header.rec_len,
        });
    }

    Ok(ConnectorRuleObservationV1 {
        record_source: record.source.clone(),
        payload_source: record.payload_source.clone(),
        ruid: read_u32(payload, 0),
        spid_a: read_u32(payload, 4),
        spid_b: read_u32(payload, 8),
        spid_c: read_u32(payload, 12),
        cpti_a: read_u32(payload, 16),
        cpti_b: read_u32(payload, 20),
    })
}

fn collect(
    records: &[OfficeArtRecord],
    bytes: &[u8],
    solvers: &mut Vec<SolverContainerObservationV1>,
    connector_shapes: &mut Vec<ConnectorShapeObservationV1>,
) -> Result<(), ConnectorReadError> {
    for record in records {
        if record.header.rec_type == OFFICE_ART_SOLVER_CONTAINER {
            if let OfficeArtBody::Container { children } = &record.body {
                let mut connector_rules = Vec::new();
                let mut unknown_children = Vec::new();
                for child in children {
                    if child.header.rec_type == OFFICE_ART_CONNECTOR_RULE {
                        connector_rules.push(decode_connector_rule(child, bytes)?);
                    } else {
                        unknown_children.push(child.source.clone());
                    }
                }
                solvers.push(SolverContainerObservationV1 {
                    source: record.source.clone(),
                    connector_rules,
                    unknown_children,
                });
            }
        }

        if record.header.rec_type == OFFICE_ART_SP_CONTAINER {
            if let OfficeArtBody::Container { children } = &record.body {
                for child in children {
                    if child.header.rec_type != OFFICE_ART_FSP {
                        continue;
                    }
                    if let OfficeArtBody::Fsp(fsp) = &child.body {
                        let f_connector = fsp.flags & FSP_CONNECTOR_BIT != 0;
                        if f_connector {
                            connector_shapes.push(ConnectorShapeObservationV1 {
                                sp_container_source: record.source.clone(),
                                fsp_source: fsp.source.clone(),
                                spid: fsp.spid,
                                raw_flags: fsp.flags,
                                f_connector,
                            });
                        }
                    }
                }
            }
        }

        if record.header.rec_type != OFFICE_ART_SOLVER_CONTAINER {
            if let OfficeArtBody::Container { children } = &record.body {
                collect(children, bytes, solvers, connector_shapes)?;
            }
        }
    }
    Ok(())
}

fn inspect(
    logical_stream: StreamPath,
    bytes: &[u8],
) -> Result<ConnectorRuleInventoryV1, ConnectorReadError> {
    let parsed = parse_officeart_stream(logical_stream.clone(), bytes)?;
    let mut solver_containers = Vec::new();
    let mut connector_shapes = Vec::new();
    collect(
        &parsed.records,
        bytes,
        &mut solver_containers,
        &mut connector_shapes,
    )?;

    Ok(ConnectorRuleInventoryV1 {
        schema_version: 1,
        logical_stream,
        stream_len: bytes.len() as u64,
        solver_containers,
        connector_shapes,
    })
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("usage: officeart-connector-rule-read FILE [LOGICAL_STREAM]")?,
    );
    let logical_stream = args
        .next()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Escher/EscherStm".to_owned());
    if args.next().is_some() {
        return Err("usage: officeart-connector-rule-read FILE [LOGICAL_STREAM]".into());
    }

    let bytes = fs::read(input)?;
    let inventory = inspect(StreamPath(logical_stream), &bytes)?;
    println!("{}", serde_json::to_string_pretty(&inventory)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(rec_ver: u8, rec_instance: u16, rec_type: u16, payload: &[u8]) -> Vec<u8> {
        let initial = (rec_instance << 4) | u16::from(rec_ver);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&initial.to_le_bytes());
        bytes.extend_from_slice(&rec_type.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn connector_rule(
        ruid: u32,
        spid_a: u32,
        spid_b: u32,
        spid_c: u32,
        cpti_a: u32,
        cpti_b: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        for value in [ruid, spid_a, spid_b, spid_c, cpti_a, cpti_b] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        record(1, 0, OFFICE_ART_CONNECTOR_RULE, &payload)
    }

    fn solver(children: &[u8]) -> Vec<u8> {
        record(0x0f, 0, OFFICE_ART_SOLVER_CONTAINER, children)
    }

    fn connector_shape(spid: u32, flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&spid.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        let fsp = record(2, 1, OFFICE_ART_FSP, &payload);
        record(0x0f, 0, OFFICE_ART_SP_CONTAINER, &fsp)
    }

    #[test]
    fn reads_connector_rule_with_zero_connection_site() {
        let rule = connector_rule(1, 0x807, 0x806, 0x80a, 0, 2);
        let bytes = solver(&rule);
        let inventory = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap();

        assert_eq!(inventory.solver_containers.len(), 1);
        let observed = &inventory.solver_containers[0].connector_rules[0];
        assert_eq!(observed.ruid, 1);
        assert_eq!(observed.spid_a, 0x807);
        assert_eq!(observed.spid_b, 0x806);
        assert_eq!(observed.spid_c, 0x80a);
        assert_eq!(observed.cpti_a, 0);
        assert_eq!(observed.cpti_b, 2);
        assert_eq!(observed.payload_source.len, 24);
    }

    #[test]
    fn preserves_unknown_solver_child_as_raw_span() {
        let rule = connector_rule(7, 1, 2, 3, 4, 5);
        let unknown = record(1, 0, 0xF014, &[0; 8]);
        let mut children = rule;
        children.extend_from_slice(&unknown);
        let bytes = solver(&children);
        let inventory = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap();

        let solver = &inventory.solver_containers[0];
        assert_eq!(solver.connector_rules.len(), 1);
        assert_eq!(solver.unknown_children.len(), 1);
        assert_eq!(solver.unknown_children[0].len, 16);
    }

    #[test]
    fn exposes_fsp_connector_bit_without_identity_join() {
        let bytes = connector_shape(0x80a, FSP_CONNECTOR_BIT);
        let inventory = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap();

        assert_eq!(inventory.connector_shapes.len(), 1);
        let shape = &inventory.connector_shapes[0];
        assert_eq!(shape.spid, 0x80a);
        assert_eq!(shape.raw_flags, FSP_CONNECTOR_BIT);
        assert!(shape.f_connector);
    }

    #[test]
    fn non_connector_fsp_is_not_reported_as_connector_shape() {
        let bytes = connector_shape(0x80a, 0);
        let inventory = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap();
        assert!(inventory.connector_shapes.is_empty());
    }

    #[test]
    fn rejects_connector_rule_with_impossible_declared_length() {
        let mut bad = connector_rule(1, 2, 3, 4, 0, 1);
        bad[4..8].copy_from_slice(&40u32.to_le_bytes());
        let bytes = solver(&bad);
        let error = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap_err();
        assert!(error.to_string().contains("за границей"));
    }

    #[test]
    fn rejects_connector_rule_with_wrong_exact_payload_length() {
        let bad = record(1, 0, OFFICE_ART_CONNECTOR_RULE, &[0; 20]);
        let bytes = solver(&bad);
        let error = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap_err();
        assert!(error.to_string().contains("recLen=24"));
    }

    #[test]
    fn rejects_truncated_solver_child_header_without_overread() {
        let bytes = solver(&[0; 7]);
        let error = inspect(StreamPath("/Escher/EscherStm".to_owned()), &bytes).unwrap_err();
        assert!(error.to_string().contains("обрезанный OfficeArt header"));
    }
}
