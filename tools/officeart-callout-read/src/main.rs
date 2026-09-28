use pub_core::{RawSpan, StreamPath};
use pub_escher::{Fopte, inspect_sp_containers};
use serde::Serialize;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

const UNUSED_832_PROPERTY_ID: u16 = 0x0340;
const DXY_CALLOUT_GAP_PROPERTY_ID: u16 = 0x0341;
const SPCOA_PROPERTY_ID: u16 = 0x0342;
const SPCOD_PROPERTY_ID: u16 = 0x0343;
const DXY_CALLOUT_DROP_SPECIFIED_PROPERTY_ID: u16 = 0x0344;
const DXY_CALLOUT_LENGTH_SPECIFIED_PROPERTY_ID: u16 = 0x0345;
const CALLOUT_BOOLEAN_PROPERTY_ID: u16 = 0x037F;

const USE_CALLOUT_BIT: u32 = 1 << 9;
const USE_CALLOUT_ACCENT_BAR_BIT: u32 = 1 << 10;
const USE_CALLOUT_TEXT_BORDER_BIT: u32 = 1 << 11;
const USE_CALLOUT_MINUS_X_BIT: u32 = 1 << 12;
const USE_CALLOUT_MINUS_Y_BIT: u32 = 1 << 13;
const USE_CALLOUT_DROP_AUTO_BIT: u32 = 1 << 14;
const USE_CALLOUT_LENGTH_SPECIFIED_BIT: u32 = 1 << 15;

const CALLOUT_BIT: u32 = 1 << 25;
const CALLOUT_ACCENT_BAR_BIT: u32 = 1 << 26;
const CALLOUT_TEXT_BORDER_BIT: u32 = 1 << 27;
const CALLOUT_MINUS_X_BIT: u32 = 1 << 28;
const CALLOUT_MINUS_Y_BIT: u32 = 1 << 29;
const CALLOUT_DROP_AUTO_BIT: u32 = 1 << 30;
const CALLOUT_LENGTH_SPECIFIED_BIT: u32 = 1 << 31;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CalloutBooleansV1 {
    f_use_callout: bool,
    f_use_callout_accent_bar: bool,
    f_use_callout_text_border: bool,
    f_use_callout_minus_x: bool,
    f_use_callout_minus_y: bool,
    f_use_callout_drop_auto: bool,
    f_use_callout_length_specified: bool,
    f_callout: bool,
    f_callout_accent_bar: bool,
    f_callout_text_border: bool,
    f_callout_minus_x: bool,
    f_callout_minus_y: bool,
    f_callout_drop_auto: bool,
    f_callout_length_specified: bool,
    callout: Option<bool>,
    callout_accent_bar: Option<bool>,
    callout_text_border: Option<bool>,
    callout_minus_x: Option<bool>,
    callout_minus_y: Option<bool>,
    callout_drop_auto: Option<bool>,
    callout_length_specified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CalloutPropertyValueV1 {
    SignedEmu {
        value: i32,
    },
    ConnectionAngle {
        raw: u32,
        known: Option<&'static str>,
    },
    ConnectionPosition {
        raw: u32,
        known: Option<&'static str>,
    },
    Booleans {
        fields: CalloutBooleansV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CalloutObservationV1 {
    shape_index: usize,
    fopt_index: usize,
    property_index: usize,
    rec_type: u16,
    property_id: u16,
    raw_op: u32,
    source: RawSpan,
    value: CalloutPropertyValueV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CalloutInventoryV1 {
    schema_version: u32,
    logical_stream: StreamPath,
    stream_len: u64,
    shapes_seen: usize,
    observations: Vec<CalloutObservationV1>,
}

fn effective(raw: u32, use_bit: u32, value_bit: u32) -> Option<bool> {
    (raw & use_bit != 0).then_some(raw & value_bit != 0)
}

fn decode_booleans(raw: u32) -> CalloutBooleansV1 {
    CalloutBooleansV1 {
        f_use_callout: raw & USE_CALLOUT_BIT != 0,
        f_use_callout_accent_bar: raw & USE_CALLOUT_ACCENT_BAR_BIT != 0,
        f_use_callout_text_border: raw & USE_CALLOUT_TEXT_BORDER_BIT != 0,
        f_use_callout_minus_x: raw & USE_CALLOUT_MINUS_X_BIT != 0,
        f_use_callout_minus_y: raw & USE_CALLOUT_MINUS_Y_BIT != 0,
        f_use_callout_drop_auto: raw & USE_CALLOUT_DROP_AUTO_BIT != 0,
        f_use_callout_length_specified: raw & USE_CALLOUT_LENGTH_SPECIFIED_BIT != 0,
        f_callout: raw & CALLOUT_BIT != 0,
        f_callout_accent_bar: raw & CALLOUT_ACCENT_BAR_BIT != 0,
        f_callout_text_border: raw & CALLOUT_TEXT_BORDER_BIT != 0,
        f_callout_minus_x: raw & CALLOUT_MINUS_X_BIT != 0,
        f_callout_minus_y: raw & CALLOUT_MINUS_Y_BIT != 0,
        f_callout_drop_auto: raw & CALLOUT_DROP_AUTO_BIT != 0,
        f_callout_length_specified: raw & CALLOUT_LENGTH_SPECIFIED_BIT != 0,
        callout: effective(raw, USE_CALLOUT_BIT, CALLOUT_BIT),
        callout_accent_bar: effective(raw, USE_CALLOUT_ACCENT_BAR_BIT, CALLOUT_ACCENT_BAR_BIT),
        callout_text_border: effective(raw, USE_CALLOUT_TEXT_BORDER_BIT, CALLOUT_TEXT_BORDER_BIT),
        callout_minus_x: effective(raw, USE_CALLOUT_MINUS_X_BIT, CALLOUT_MINUS_X_BIT),
        callout_minus_y: effective(raw, USE_CALLOUT_MINUS_Y_BIT, CALLOUT_MINUS_Y_BIT),
        callout_drop_auto: effective(raw, USE_CALLOUT_DROP_AUTO_BIT, CALLOUT_DROP_AUTO_BIT),
        callout_length_specified: effective(
            raw,
            USE_CALLOUT_LENGTH_SPECIFIED_BIT,
            CALLOUT_LENGTH_SPECIFIED_BIT,
        ),
    }
}

fn spcoa_name(raw: u32) -> Option<&'static str> {
    match raw {
        0 => Some("msospcoaAny"),
        1 => Some("msospcoa30"),
        2 => Some("msospcoa45"),
        3 => Some("msospcoa60"),
        4 => Some("msospcoa90"),
        5 => Some("msospcoa0"),
        _ => None,
    }
}

fn spcod_name(raw: u32) -> Option<&'static str> {
    match raw {
        0 => Some("msospcodTop"),
        1 => Some("msospcodCenter"),
        2 => Some("msospcodBottom"),
        3 => Some("msospcodSpecified"),
        _ => None,
    }
}

fn decode_callout(entry: &Fopte) -> Option<CalloutPropertyValueV1> {
    if entry.f_bid() || entry.f_complex() {
        return None;
    }

    match entry.property_id() {
        DXY_CALLOUT_GAP_PROPERTY_ID
        | DXY_CALLOUT_DROP_SPECIFIED_PROPERTY_ID
        | DXY_CALLOUT_LENGTH_SPECIFIED_PROPERTY_ID => Some(CalloutPropertyValueV1::SignedEmu {
            value: entry.op as i32,
        }),
        SPCOA_PROPERTY_ID => Some(CalloutPropertyValueV1::ConnectionAngle {
            raw: entry.op,
            known: spcoa_name(entry.op),
        }),
        SPCOD_PROPERTY_ID => Some(CalloutPropertyValueV1::ConnectionPosition {
            raw: entry.op,
            known: spcod_name(entry.op),
        }),
        CALLOUT_BOOLEAN_PROPERTY_ID => Some(CalloutPropertyValueV1::Booleans {
            fields: decode_booleans(entry.op),
        }),
        _ => None,
    }
}

fn inspect(logical_stream: StreamPath, bytes: &[u8]) -> Result<CalloutInventoryV1, Box<dyn Error>> {
    let shapes = inspect_sp_containers(logical_stream.clone(), bytes)?;
    let mut observations = Vec::new();

    for (shape_index, shape) in shapes.shapes.iter().enumerate() {
        for (fopt_index, fopt) in shape.fopts.iter().enumerate() {
            for (property_index, entry) in fopt.properties.iter().enumerate() {
                let Some(value) = decode_callout(entry) else {
                    continue;
                };
                observations.push(CalloutObservationV1 {
                    shape_index,
                    fopt_index,
                    property_index,
                    rec_type: fopt.rec_type,
                    property_id: entry.property_id(),
                    raw_op: entry.op,
                    source: entry.source.clone(),
                    value,
                });
            }
        }
    }

    Ok(CalloutInventoryV1 {
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
            .ok_or("usage: officeart-callout-read FILE [LOGICAL_STREAM]")?,
    );
    let logical_stream = args
        .next()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Escher/EscherStm".to_owned());
    if args.next().is_some() {
        return Err("usage: officeart-callout-read FILE [LOGICAL_STREAM]".into());
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

    fn one(opid: u16, op: u32) -> CalloutInventoryV1 {
        let mut fopt_payload = Vec::new();
        fopt_payload.extend_from_slice(&opid.to_le_bytes());
        fopt_payload.extend_from_slice(&op.to_le_bytes());
        let fopt = header((1 << 4) | 0x3, pub_escher::OFFICE_ART_FOPT, &fopt_payload);
        let sp = header(0x000f, pub_escher::OFFICE_ART_SP_CONTAINER, &fopt);
        inspect(StreamPath("/Escher/EscherStm".to_owned()), &sp)
            .expect("synthetic OfficeArt should parse")
    }

    fn boolean_states(fields: &CalloutBooleansV1) -> [(bool, bool, Option<bool>); 7] {
        [
            (fields.f_use_callout, fields.f_callout, fields.callout),
            (
                fields.f_use_callout_accent_bar,
                fields.f_callout_accent_bar,
                fields.callout_accent_bar,
            ),
            (
                fields.f_use_callout_text_border,
                fields.f_callout_text_border,
                fields.callout_text_border,
            ),
            (
                fields.f_use_callout_minus_x,
                fields.f_callout_minus_x,
                fields.callout_minus_x,
            ),
            (
                fields.f_use_callout_minus_y,
                fields.f_callout_minus_y,
                fields.callout_minus_y,
            ),
            (
                fields.f_use_callout_drop_auto,
                fields.f_callout_drop_auto,
                fields.callout_drop_auto,
            ),
            (
                fields.f_use_callout_length_specified,
                fields.f_callout_length_specified,
                fields.callout_length_specified,
            ),
        ]
    }

    fn boolean_fields(inventory: &CalloutInventoryV1) -> &CalloutBooleansV1 {
        match &inventory.observations[0].value {
            CalloutPropertyValueV1::Booleans { fields } => fields,
            other => panic!("expected booleans, got {other:?}"),
        }
    }

    #[test]
    fn current_unused_832_is_not_promoted_as_callout_type() {
        let inventory = one(UNUSED_832_PROPERTY_ID, 4);
        assert!(inventory.observations.is_empty());
    }

    #[test]
    fn signed_emu_properties_preserve_raw_sign() {
        for property_id in [
            DXY_CALLOUT_GAP_PROPERTY_ID,
            DXY_CALLOUT_DROP_SPECIFIED_PROPERTY_ID,
            DXY_CALLOUT_LENGTH_SPECIFIED_PROPERTY_ID,
        ] {
            let inventory = one(property_id, u32::MAX);
            let observation = &inventory.observations[0];
            assert_eq!(observation.property_id, property_id);
            assert_eq!(observation.raw_op, u32::MAX);
            assert_eq!(
                observation.value,
                CalloutPropertyValueV1::SignedEmu { value: -1 }
            );
        }
    }

    #[test]
    fn spcoa_known_and_unknown_values_remain_distinct() {
        let known = one(SPCOA_PROPERTY_ID, 2);
        assert_eq!(
            known.observations[0].value,
            CalloutPropertyValueV1::ConnectionAngle {
                raw: 2,
                known: Some("msospcoa45"),
            }
        );

        let unknown = one(SPCOA_PROPERTY_ID, 99);
        assert_eq!(
            unknown.observations[0].value,
            CalloutPropertyValueV1::ConnectionAngle {
                raw: 99,
                known: None,
            }
        );
    }

    #[test]
    fn spcod_known_and_unknown_values_remain_distinct() {
        let known = one(SPCOD_PROPERTY_ID, 3);
        assert_eq!(
            known.observations[0].value,
            CalloutPropertyValueV1::ConnectionPosition {
                raw: 3,
                known: Some("msospcodSpecified"),
            }
        );

        let unknown = one(SPCOD_PROPERTY_ID, 99);
        assert_eq!(
            unknown.observations[0].value,
            CalloutPropertyValueV1::ConnectionPosition {
                raw: 99,
                known: None,
            }
        );
    }

    #[test]
    fn every_boolean_pair_preserves_absent_false_and_true() {
        let pairs = [
            (USE_CALLOUT_BIT, CALLOUT_BIT),
            (USE_CALLOUT_ACCENT_BAR_BIT, CALLOUT_ACCENT_BAR_BIT),
            (USE_CALLOUT_TEXT_BORDER_BIT, CALLOUT_TEXT_BORDER_BIT),
            (USE_CALLOUT_MINUS_X_BIT, CALLOUT_MINUS_X_BIT),
            (USE_CALLOUT_MINUS_Y_BIT, CALLOUT_MINUS_Y_BIT),
            (USE_CALLOUT_DROP_AUTO_BIT, CALLOUT_DROP_AUTO_BIT),
            (
                USE_CALLOUT_LENGTH_SPECIFIED_BIT,
                CALLOUT_LENGTH_SPECIFIED_BIT,
            ),
        ];

        for (index, (use_bit, value_bit)) in pairs.into_iter().enumerate() {
            let absent = one(CALLOUT_BOOLEAN_PROPERTY_ID, value_bit);
            assert_eq!(
                boolean_states(boolean_fields(&absent))[index],
                (false, true, None)
            );

            let explicit_false = one(CALLOUT_BOOLEAN_PROPERTY_ID, use_bit);
            assert_eq!(
                boolean_states(boolean_fields(&explicit_false))[index],
                (true, false, Some(false))
            );

            let explicit_true = one(CALLOUT_BOOLEAN_PROPERTY_ID, use_bit | value_bit);
            assert_eq!(
                boolean_states(boolean_fields(&explicit_true))[index],
                (true, true, Some(true))
            );
        }
    }

    #[test]
    fn neighboring_unused_bits_do_not_alias_callout_booleans() {
        let inventory = one(CALLOUT_BOOLEAN_PROPERTY_ID, (1 << 8) | (1 << 24));
        assert_eq!(
            boolean_states(boolean_fields(&inventory)),
            [(false, false, None); 7]
        );
    }

    #[test]
    fn observation_preserves_raw_fopte_provenance() {
        let inventory = one(DXY_CALLOUT_GAP_PROPERTY_ID, 0x1234);
        assert_eq!(inventory.schema_version, 1);
        assert_eq!(inventory.shapes_seen, 1);
        assert_eq!(inventory.stream_len, 22);
        let observation = &inventory.observations[0];
        assert_eq!(observation.rec_type, pub_escher::OFFICE_ART_FOPT);
        assert_eq!(observation.property_id, DXY_CALLOUT_GAP_PROPERTY_ID);
        assert_eq!(observation.raw_op, 0x1234);
        assert_eq!(observation.source.offset, 16);
        assert_eq!(observation.source.len, 6);
        assert_eq!(observation.shape_index, 0);
        assert_eq!(observation.fopt_index, 0);
        assert_eq!(observation.property_index, 0);
    }

    #[test]
    fn fbid_and_complex_variants_are_not_promoted_as_scalar_callout_properties() {
        let fbid = one(DXY_CALLOUT_GAP_PROPERTY_ID | 0x4000, 1);
        assert!(fbid.observations.is_empty());

        let complex = one(DXY_CALLOUT_GAP_PROPERTY_ID | 0x8000, 1);
        assert!(complex.observations.is_empty());
    }
}
