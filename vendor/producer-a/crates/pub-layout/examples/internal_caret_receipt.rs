use pub_layout::{
    BoundedLayoutEnvironment, BoundedShapingRuntime, font_fingerprint_sha256,
    resolve_internal_carets_ltr,
};
use pub_model::{EMU_PER_POINT, LengthEmu};

fn main() {
    let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
    let runtime = BoundedShapingRuntime {
        layout: BoundedLayoutEnvironment {
            engine_revision: "layout-text-internal-caret-auth-01".into(),
            font_set_fingerprint: font_fingerprint_sha256(font),
            resource_fingerprint: "resources:none".into(),
        },
        face_index: 0,
        font_size_emu: LengthEmu::new(12 * EMU_PER_POINT),
        font_bytes: font,
    };
    let result = resolve_internal_carets_ltr("fi", 100, &runtime, &[101])
        .expect("pinned fi GDEF caret authority");
    println!("{}", serde_json::to_string_pretty(&result).expect("serialize receipt"));
}
