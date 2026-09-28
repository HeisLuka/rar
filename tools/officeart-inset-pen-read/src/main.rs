use pub_core::{RawSpan, StreamPath};
use pub_escher::{Fopte, inspect_sp_containers};
use serde::Serialize;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

const LINE_STYLE_BOOLEAN_PROPERTY_ID: u16 = 0x01FF;
const USE_INSET_PEN_BIT: u32 = 1 << 9;
const USE_INSET_PEN_OK_BIT: u32 = 1 << 10;
const INSET_PEN_BIT: u32 = 1 << 25;
const INSET_PEN_OK_BIT: u32 = 1 << 26;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct InsetPenObservationV1 {
    shape_index: usize,
    fopt_index: usize,
    property_index: usize,
    rec_type: u16,
    raw_op: u32,
    source: RawSpan,
    f_use_inset_pen: bool,
    f_use_inset_pen_ok: bool,
    f_inset_pen: bool,
    f_inset_pen_ok: bool,
    inset_pen: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct InsetPenInventoryV1 {
    schema_version: u32,
    logical_stream: StreamPath,
    stream_len: u64,
    shapes_seen: usize,
    observations: Vec<InsetPenObservationV1>,
}

fn decode_inset_pen(entry: &Fopte) -> Option<(bool, bool, bool, bool, Option<bool>)> {
    if entry.property_id() != LINE_STYLE_BOOLEAN_PROPERTY_ID || entry.f_bid() || entry.f_complex() {
        return None;
    }

    let use_inset = entry.op & USE_INSET_PEN_BIT != 0;
    let use_ok = entry.op & USE_INSET_PEN_OK_BIT != 0;
    let inset = entry.op & INSET_PEN_BIT != 0;
    let inset_ok = entry.op & INSET_PEN_OK_BIT != 0;

    // Effective observation is only admitted when both use gates are explicit
    // and the OfficeArt "OK" value allows the property.
    let effective = (use_inset && use_ok && inset_ok).then_some(inset);
    Some((use_inset, use_ok, inset, inset_ok, effective))
}

fn inspect(
    logical_stream: StreamPath,
    bytes: &[u8],
) -> Result<InsetPenInventoryV1, Box<dyn Error>> {
    let shapes = inspect_sp_containers(logical_stream.clone(), bytes)?;
    let mut observations = Vec::new();

    for (shape_index, shape) in shapes.shapes.iter().enumerate() {
        for (fopt_index, fopt) in shape.fopts.iter().enumerate() {
            for (property_index, entry) in fopt.properties.iter().enumerate() {
                let Some((use_inset, use_ok, inset, inset_ok, effective)) = decode_inset_pen(entry)
                else {
                    continue;
                };
                observations.push(InsetPenObservationV1 {
                    shape_index,
                    fopt_index,
                    property_index,
                    rec_type: fopt.rec_type,
                    raw_op: entry.op,
                    source: entry.source.clone(),
                    f_use_inset_pen: use_inset,
                    f_use_inset_pen_ok: use_ok,
                    f_inset_pen: inset,
                    f_inset_pen_ok: inset_ok,
                    inset_pen: effective,
                });
            }
        }
    }

    Ok(InsetPenInventoryV1 {
        schema_version: 1,
        logical_stream,
        stream_len: shapes.stream_len,
        shapes_seen: shapes.shapes.len(),
        observations,
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
            .ok_or("usage: officeart-inset-pen-read FILE [LOGICAL_STREAM]")?,
    );
    let logical_stream = args
        .next()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Escher/EscherStm".to_owned());
    if args.next().is_some() {
        return Err("usage: officeart-inset-pen-read FILE [LOGICAL_STREAM]".into());
    }

    let bytes = fs::read(input)?;
    let inventory = inspect(StreamPath(logical_stream), &bytes)?;
    println!("{}", serde_json::to_string_pretty(&inventory)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(initial: u16, rec_type: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&initial.to_le_bytes());
        bytes.extend_from_slice(&rec_type.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn one(opid: u16, op: u32) -> InsetPenInventoryV1 {
        let mut fopt_payload = Vec::new();
        fopt_payload.extend_from_slice(&opid.to_le_bytes());
        fopt_payload.extend_from_slice(&op.to_le_bytes());
        let fopt = header((1 << 4) | 0x3, pub_escher::OFFICE_ART_FOPT, &fopt_payload);
        let sp = header(0x000f, pub_escher::OFFICE_ART_SP_CONTAINER, &fopt);
        inspect(StreamPath("/Escher/EscherStm".to_owned()), &sp)
            .expect("synthetic OfficeArt should parse")
    }

    #[test]
    fn absent_use_gate_never_materializes_false_or_true() {
        let inventory = one(
            LINE_STYLE_BOOLEAN_PROPERTY_ID,
            INSET_PEN_BIT | INSET_PEN_OK_BIT,
        );
        let o = &inventory.observations[0];
        assert!(!o.f_use_inset_pen);
        assert!(!o.f_use_inset_pen_ok);
        assert!(o.f_inset_pen);
        assert!(o.f_inset_pen_ok);
        assert_eq!(o.inset_pen, None);
    }

    #[test]
    fn explicit_false_is_admitted_when_use_and_ok_are_explicit() {
        let inventory = one(
            LINE_STYLE_BOOLEAN_PROPERTY_ID,
            USE_INSET_PEN_BIT | USE_INSET_PEN_OK_BIT | INSET_PEN_OK_BIT,
        );
        let o = &inventory.observations[0];
        assert_eq!(o.inset_pen, Some(false));
    }

    #[test]
    fn explicit_true_is_admitted_when_use_and_ok_are_explicit() {
        let inventory = one(
            LINE_STYLE_BOOLEAN_PROPERTY_ID,
            USE_INSET_PEN_BIT | USE_INSET_PEN_OK_BIT | INSET_PEN_BIT | INSET_PEN_OK_BIT,
        );
        let o = &inventory.observations[0];
        assert_eq!(o.inset_pen, Some(true));
    }

    #[test]
    fn ok_use_gate_absent_keeps_effective_value_unresolved() {
        let inventory = one(
            LINE_STYLE_BOOLEAN_PROPERTY_ID,
            USE_INSET_PEN_BIT | INSET_PEN_BIT | INSET_PEN_OK_BIT,
        );
        let o = &inventory.observations[0];
        assert!(o.f_use_inset_pen);
        assert!(!o.f_use_inset_pen_ok);
        assert!(o.f_inset_pen);
        assert!(o.f_inset_pen_ok);
        assert_eq!(o.inset_pen, None);
    }

    #[test]
    fn explicit_ok_false_blocks_effective_inset_pen() {
        let inventory = one(
            LINE_STYLE_BOOLEAN_PROPERTY_ID,
            USE_INSET_PEN_BIT | USE_INSET_PEN_OK_BIT | INSET_PEN_BIT,
        );
        let o = &inventory.observations[0];
        assert!(o.f_use_inset_pen);
        assert!(o.f_use_inset_pen_ok);
        assert!(o.f_inset_pen);
        assert!(!o.f_inset_pen_ok);
        assert_eq!(o.inset_pen, None);
    }

    #[test]
    fn observation_preserves_raw_fopte_provenance() {
        let raw = USE_INSET_PEN_BIT | USE_INSET_PEN_OK_BIT | INSET_PEN_OK_BIT;
        let inventory = one(LINE_STYLE_BOOLEAN_PROPERTY_ID, raw);
        assert_eq!(inventory.schema_version, 1);
        assert_eq!(inventory.shapes_seen, 1);
        assert_eq!(inventory.stream_len, 22);
        let o = &inventory.observations[0];
        assert_eq!(o.raw_op, raw);
        assert_eq!(o.rec_type, pub_escher::OFFICE_ART_FOPT);
        assert_eq!(o.source.offset, 16);
        assert_eq!(o.source.len, 6);
    }

    #[test]
    fn fbid_variant_is_not_promoted_as_scalar_line_boolean() {
        let inventory = one(
            LINE_STYLE_BOOLEAN_PROPERTY_ID | 0x4000,
            USE_INSET_PEN_BIT | USE_INSET_PEN_OK_BIT | INSET_PEN_BIT | INSET_PEN_OK_BIT,
        );
        assert!(inventory.observations.is_empty());
    }
}
