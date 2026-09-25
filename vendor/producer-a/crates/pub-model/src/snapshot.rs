use serde::Serialize;
use serde_json::Value;

pub const CDM_DEBUG_JSON_PROFILE_V0_1: &str = "cdm-debug-json-v0.1";

/// Serializes a CDM/debug value with deterministic JSON object-key ordering.
///
/// Arrays are emitted in their semantic input order. JSON object keys are
/// sorted recursively. No Unicode normalization or volatile metadata is added.
pub fn to_cdm_debug_json_v0_1<T: Serialize + ?Sized>(
    value: &T,
) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    let mut out = Vec::new();
    write_canonical_value(&mut out, &value)?;
    Ok(out)
}

fn write_canonical_value(out: &mut Vec<u8>, value: &Value) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(map) => {
            out.push(b'{');
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical_value(out, &map[key])?;
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical_value(out, item)?;
            }
            out.push(b']');
        }
        scalar => serde_json::to_writer(out, scalar)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use std::collections::HashMap;

    #[derive(Serialize)]
    struct Fixture {
        semantic_order: Vec<u32>,
        unordered: HashMap<String, u32>,
    }

    #[test]
    fn object_keys_are_canonical_but_array_order_is_preserved() {
        let mut left = HashMap::new();
        left.insert("z".into(), 1);
        left.insert("a".into(), 2);

        let mut right = HashMap::new();
        right.insert("a".into(), 2);
        right.insert("z".into(), 1);

        let a = to_cdm_debug_json_v0_1(&Fixture {
            semantic_order: vec![3, 1, 2],
            unordered: left,
        })
        .unwrap();
        let b = to_cdm_debug_json_v0_1(&Fixture {
            semantic_order: vec![3, 1, 2],
            unordered: right,
        })
        .unwrap();

        assert_eq!(a, b);
        assert_eq!(
            String::from_utf8(a).unwrap(),
            r#"{"semantic_order":[3,1,2],"unordered":{"a":2,"z":1}}"#
        );
    }

    #[test]
    fn unicode_scalars_are_not_normalized() {
        let composed = to_cdm_debug_json_v0_1(&"é").unwrap();
        let decomposed = to_cdm_debug_json_v0_1(&"e\u{301}").unwrap();

        assert_ne!(composed, decomposed);
    }
}
