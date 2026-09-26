use chaptera_text_caret_map_adapter::{
    build_resolved_text_caret_map_v1, caret_map_hash_v1, CaretMapBuildInputV1,
};
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("read stdin");
    let parsed: CaretMapBuildInputV1 = serde_json::from_str(&input).expect("parse input");
    let map = build_resolved_text_caret_map_v1(parsed).expect("build caret map");
    println!("{}", caret_map_hash_v1(&map));
}
