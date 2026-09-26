use chaptera_text_caret_map_adapter::{
    CaretMapBuildInputV1, build_resolved_text_caret_map_v1, caret_map_to_value_v1,
};
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("read stdin");
    let parsed: CaretMapBuildInputV1 = serde_json::from_str(&input).expect("parse input");
    let map = build_resolved_text_caret_map_v1(parsed).expect("build caret map");
    println!(
        "{}",
        serde_json::to_string(&caret_map_to_value_v1(&map)).expect("serialize")
    );
}
