use pub_core::{RawSpan, StreamPath};
use pub_escher::{Fopte, inspect_sp_containers};
use serde::Serialize;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

const PROTECTION_BOOLEAN_PROPERTY_ID: u16 = 0x007F;
const USE_LOCK_ROTATION_BIT: u32 = 1 << 7;
const USE_LOCK_ASPECT_RATIO_BIT: u32 = 1 << 8;
const LOCK_ROTATION_BIT: u32 = 1 << 23;
const LOCK_ASPECT_RATIO_BIT: u32 = 1 << 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AspectLockObservationV1 {
    shape_index: usize,
    fopt_index: usize,
    property_index: usize,
    rec_type: u16,
    raw_op: u32,
    source: RawSpan,
    f_use_lock_aspect_ratio: bool,
    f_lock_aspect_ratio: bool,
    aspect_lock: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AspectLockInventoryV1 {
    schema_version: u32,
    logical_stream: StreamPath,
    stream_len: u64,
    shapes_seen: usize,
    observations: Vec<AspectLockObservationV1>,
}

fn decode_aspect_lock(entry: &Fopte) -> Option<(bool, bool, Option<bool>)> {
    if entry.property_id() != PROTECTION_BOOLEAN_PROPERTY_ID || entry.f_bid() || entry.f_complex() {
        return None;
    }

    let use_lock = entry.op & USE_LOCK_ASPECT_RATIO_BIT != 0;
    let lock = entry.op & LOCK_ASPECT_RATIO_BIT != 0;
    Some((use_lock, lock, use_lock.then_some(lock)))
}

fn inspect(
    logical_stream: StreamPath,
    bytes: &[u8],
) -> Result<AspectLockInventoryV1, Box<dyn Error>> {
    let shapes = inspect_sp_containers(logical_stream.clone(), bytes)?;
    let mut observations = Vec::new();

    for (shape_index, shape) in shapes.shapes.iter().enumerate() {
        for (fopt_index, fopt) in shape.fopts.iter().enumerate() {
            for (property_index, entry) in fopt.properties.iter().enumerate() {
                let Some((use_lock, lock, aspect_lock)) = decode_aspect_lock(entry) else {
                    continue;
                };
                observations.push(AspectLockObservationV1 {
                    shape_index,
                    fopt_index,
                    property_index,
                    rec_type: fopt.rec_type,
                    raw_op: entry.op,
                    source: entry.source.clone(),
                    f_use_lock_aspect_ratio: use_lock,
                    f_lock_aspect_ratio: lock,
                    aspect_lock,
                });
            }
        }
    }

    Ok(AspectLockInventoryV1 {
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
            .ok_or("usage: officeart-aspect-lock-read FILE [LOGICAL_STREAM]")?,
    );
    let logical_stream = args
        .next()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Escher/EscherStm".to_owned());
    if args.next().is_some() {
        return Err("usage: officeart-aspect-lock-read FILE [LOGICAL_STREAM]".into());
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

    fn fopt_property(opid: u16, op: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&opid.to_le_bytes());
        payload.extend_from_slice(&op.to_le_bytes());
        header((1 << 4) | 0x3, pub_escher::OFFICE_ART_FOPT, &payload)
    }

    fn sp_container_with_property(opid: u16, op: u32) -> Vec<u8> {
        let fopt = fopt_property(opid, op);
        header(0x000f, pub_escher::OFFICE_ART_SP_CONTAINER, &fopt)
    }

    fn one(opid: u16, op: u32) -> AspectLockInventoryV1 {
        inspect(
            StreamPath("/Escher/EscherStm".to_owned()),
            &sp_container_with_property(opid, op),
        )
        .expect("synthetic OfficeArt should parse")
    }

    #[test]
    fn use_absent_stays_unresolved_even_when_raw_value_bit_is_one() {
        let inventory = one(PROTECTION_BOOLEAN_PROPERTY_ID, LOCK_ASPECT_RATIO_BIT);
        let observation = &inventory.observations[0];
        assert!(!observation.f_use_lock_aspect_ratio);
        assert!(observation.f_lock_aspect_ratio);
        assert_eq!(observation.aspect_lock, None);
    }

    #[test]
    fn explicit_false_requires_use_bit() {
        let inventory = one(PROTECTION_BOOLEAN_PROPERTY_ID, USE_LOCK_ASPECT_RATIO_BIT);
        let observation = &inventory.observations[0];
        assert!(observation.f_use_lock_aspect_ratio);
        assert!(!observation.f_lock_aspect_ratio);
        assert_eq!(observation.aspect_lock, Some(false));
    }

    #[test]
    fn explicit_true_requires_use_and_value_bits() {
        let inventory = one(
            PROTECTION_BOOLEAN_PROPERTY_ID,
            USE_LOCK_ASPECT_RATIO_BIT | LOCK_ASPECT_RATIO_BIT,
        );
        let observation = &inventory.observations[0];
        assert!(observation.f_use_lock_aspect_ratio);
        assert!(observation.f_lock_aspect_ratio);
        assert_eq!(observation.aspect_lock, Some(true));
    }

    #[test]
    fn neighboring_rotation_bits_do_not_alias_aspect_lock() {
        let inventory = one(
            PROTECTION_BOOLEAN_PROPERTY_ID,
            USE_LOCK_ROTATION_BIT | LOCK_ROTATION_BIT,
        );
        let observation = &inventory.observations[0];
        assert!(!observation.f_use_lock_aspect_ratio);
        assert!(!observation.f_lock_aspect_ratio);
        assert_eq!(observation.aspect_lock, None);
    }

    #[test]
    fn observation_preserves_raw_fopte_provenance() {
        let raw = USE_LOCK_ASPECT_RATIO_BIT | LOCK_ASPECT_RATIO_BIT;
        let inventory = one(PROTECTION_BOOLEAN_PROPERTY_ID, raw);
        assert_eq!(inventory.schema_version, 1);
        assert_eq!(inventory.shapes_seen, 1);
        assert_eq!(inventory.stream_len, 22);
        let observation = &inventory.observations[0];
        assert_eq!(observation.raw_op, raw);
        assert_eq!(observation.rec_type, pub_escher::OFFICE_ART_FOPT);
        assert_eq!(observation.source.offset, 16);
        assert_eq!(observation.source.len, 6);
        assert_eq!(observation.shape_index, 0);
        assert_eq!(observation.fopt_index, 0);
        assert_eq!(observation.property_index, 0);
    }

    #[test]
    fn fbid_variant_is_not_promoted_as_scalar_protection_boolean() {
        let inventory = one(
            PROTECTION_BOOLEAN_PROPERTY_ID | 0x4000,
            USE_LOCK_ASPECT_RATIO_BIT | LOCK_ASPECT_RATIO_BIT,
        );
        assert!(inventory.observations.is_empty());
    }
}
