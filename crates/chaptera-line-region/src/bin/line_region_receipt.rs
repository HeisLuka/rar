use chaptera_line_region::{
    ColumnsV1, EffectiveWrapModeV1, EffectiveWrapObstacleV1, FrameRegionV1,
    InsetsEmuV1, LineBandRequestV1, RectEmuV1, WrapDistancesV1,
    resolve_line_band_v1,
};
use std::time::Instant;

fn frame(width: i64) -> FrameRegionV1 {
    FrameRegionV1 {
        frame_id: "bench-frame".into(),
        page_id: "bench-page".into(),
        bounds: RectEmuV1 {
            x: 0,
            y: 0,
            width,
            height: 1_000_000,
        },
        insets: InsetsEmuV1 {
            left: 10_000,
            top: 10_000,
            right: 10_000,
            bottom: 10_000,
        },
    }
}

fn obstacle(x: i64) -> EffectiveWrapObstacleV1 {
    EffectiveWrapObstacleV1 {
        obstacle_id: "bench-obstacle".into(),
        mode: EffectiveWrapModeV1::Rectangle {
            rect: RectEmuV1 {
                x,
                y: 20_000,
                width: 80_000,
                height: 60_000,
            },
            distances: WrapDistancesV1 {
                left: 5_000,
                top: 5_000,
                right: 5_000,
                bottom: 5_000,
            },
        },
    }
}

fn main() {
    let columns = ColumnsV1 {
        count: 2,
        gutter_emu: 12_000,
    };
    let band = LineBandRequestV1 {
        top_emu: 30_000,
        bottom_emu: 45_000,
    };
    let iterations = 10_000u32;

    let drag_start = Instant::now();
    let mut drag_interval_count = 0usize;
    for step in 0..iterations {
        let x = 30_000 + i64::from(step % 200) * 500;
        let slots = resolve_line_band_v1(
            &frame(900_000),
            columns,
            &[obstacle(x)],
            band,
        )
        .expect("obstacle drag solve");
        drag_interval_count += slots.intervals.len();
    }
    let drag_ns = drag_start.elapsed().as_nanos();

    let resize_start = Instant::now();
    let mut resize_interval_count = 0usize;
    for step in 0..iterations {
        let width = 700_000 + i64::from(step % 200) * 1_000;
        let slots = resolve_line_band_v1(
            &frame(width),
            columns,
            &[obstacle(200_000)],
            band,
        )
        .expect("frame resize solve");
        resize_interval_count += slots.intervals.len();
    }
    let resize_ns = resize_start.elapsed().as_nanos();

    println!(
        "{{\"schema\":\"chaptera.line-region-benchmark.v1\",\"iterations_per_arm\":{},\"obstacle_drag_ns\":{},\"frame_resize_ns\":{},\"obstacle_drag_interval_count\":{},\"frame_resize_interval_count\":{},\"shaping_calls\":0,\"story_mutations\":0,\"raw_source_accesses\":0}}",
        iterations,
        drag_ns,
        resize_ns,
        drag_interval_count,
        resize_interval_count
    );
}
