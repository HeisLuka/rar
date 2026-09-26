use chaptera_text_caret_map_adapter::{
    build_resolved_text_caret_map_v1, hit_test_story_position_v1, resolve_story_position_v1,
    selection_geometry_v1, CaretMapBuildInputV1,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read};

#[derive(Debug, Deserialize)]
struct ProbeEnvelope {
    input: CaretMapBuildInputV1,
    queries: Vec<ProbeQuery>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProbeQuery {
    Resolve {
        scalar_boundary: u32,
        #[serde(default)]
        stop_id: Option<String>,
        #[serde(default)]
        expected_layout_revision_id: Option<String>,
    },
    HitTest {
        page_id: String,
        page_x_emu: i64,
        page_y_emu: i64,
        #[serde(default)]
        expected_layout_revision_id: Option<String>,
    },
    Selection {
        start_scalar: u32,
        end_scalar: u32,
        #[serde(default)]
        expected_layout_revision_id: Option<String>,
    },
}

fn result_value<T: serde::Serialize>(
    result: Result<T, chaptera_text_caret_map_adapter::CaretMapError>,
) -> Value {
    match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => json!({"error_code": error.code}),
    }
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("read stdin");
    let parsed: ProbeEnvelope = serde_json::from_str(&input).expect("parse probe envelope");
    let map = build_resolved_text_caret_map_v1(parsed.input).expect("build caret map");

    let results = parsed
        .queries
        .into_iter()
        .map(|query| match query {
            ProbeQuery::Resolve {
                scalar_boundary,
                stop_id,
                expected_layout_revision_id,
            } => result_value(resolve_story_position_v1(
                &map,
                scalar_boundary,
                stop_id.as_deref(),
                expected_layout_revision_id.as_deref(),
            )),
            ProbeQuery::HitTest {
                page_id,
                page_x_emu,
                page_y_emu,
                expected_layout_revision_id,
            } => result_value(hit_test_story_position_v1(
                &map,
                &page_id,
                page_x_emu,
                page_y_emu,
                expected_layout_revision_id.as_deref(),
            )),
            ProbeQuery::Selection {
                start_scalar,
                end_scalar,
                expected_layout_revision_id,
            } => result_value(selection_geometry_v1(
                &map,
                start_scalar,
                end_scalar,
                expected_layout_revision_id.as_deref(),
            )),
        })
        .collect::<Vec<_>>();

    println!(
        "{}",
        serde_json::to_string(&json!({"results": results})).expect("serialize probe result")
    );
}
