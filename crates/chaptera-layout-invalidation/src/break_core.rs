//! Shared non-materializing line-break decision core.
//!
//! This module owns only the deterministic choice of the next scalar-metric
//! boundary for a fixed line capacity. Story/frame materialization stays in
//! callers. Both simple StoryFlow and linked-frame flow must consume this
//! exact decision surface so probe and materialized paths cannot drift.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineBreakDecisionV1 {
    /// Exclusive metric index selected for this line.
    pub end_index: usize,
    pub measured_width_emu: i64,
    /// True when the selected line consumes the remainder of the supplied
    /// prepared metric slice.
    pub reaches_metric_end: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBreakProbeV1 {
    Selected(LineBreakDecisionV1),
    Unbreakable,
}

/// Probe the next line boundary without materializing glyphs or flow objects.
///
/// The algorithm is deliberately generic over the prepared metric type so
/// callers do not allocate adapter copies. A line may end at the end of the
/// metric slice even when the final metric is not an explicit break
/// opportunity. Otherwise the last fitting explicit break is selected.
pub fn probe_line_break_v1<T, Advance, BreakAfter>(
    metrics: &[T],
    start_index: usize,
    capacity_emu: i64,
    advance_emu: Advance,
    break_after: BreakAfter,
) -> LineBreakProbeV1
where
    Advance: Fn(&T) -> i64,
    BreakAfter: Fn(&T) -> bool,
{
    if start_index >= metrics.len() || capacity_emu <= 0 {
        return LineBreakProbeV1::Unbreakable;
    }

    let mut width = 0_i64;
    let mut probe = start_index;
    let mut last_break = None;

    while probe < metrics.len() {
        let next = width.saturating_add(advance_emu(&metrics[probe]));
        if next > capacity_emu {
            break;
        }
        width = next;
        probe += 1;
        if break_after(&metrics[probe - 1]) || probe == metrics.len() {
            last_break = Some(probe);
        }
    }

    let reaches_metric_end = probe == metrics.len();
    let end_index = if reaches_metric_end {
        probe
    } else if let Some(last_break) = last_break {
        last_break
    } else {
        return LineBreakProbeV1::Unbreakable;
    };

    if end_index <= start_index {
        return LineBreakProbeV1::Unbreakable;
    }

    let measured_width_emu = metrics[start_index..end_index]
        .iter()
        .map(&advance_emu)
        .sum();

    LineBreakProbeV1::Selected(LineBreakDecisionV1 {
        end_index,
        measured_width_emu,
        reaches_metric_end: end_index == metrics.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Metric {
        advance: i64,
        break_after: bool,
    }

    fn probe(metrics: &[Metric], start: usize, capacity: i64) -> LineBreakProbeV1 {
        probe_line_break_v1(
            metrics,
            start,
            capacity,
            |metric| metric.advance,
            |metric| metric.break_after,
        )
    }

    #[test]
    fn end_of_prepared_slice_is_a_valid_terminal_boundary() {
        let metrics = [
            Metric {
                advance: 10,
                break_after: false,
            },
            Metric {
                advance: 10,
                break_after: false,
            },
        ];

        assert_eq!(
            probe(&metrics, 0, 20),
            LineBreakProbeV1::Selected(LineBreakDecisionV1 {
                end_index: 2,
                measured_width_emu: 20,
                reaches_metric_end: true,
            })
        );
    }

    #[test]
    fn overflow_uses_last_fitting_explicit_break() {
        let metrics = [
            Metric {
                advance: 10,
                break_after: false,
            },
            Metric {
                advance: 10,
                break_after: true,
            },
            Metric {
                advance: 10,
                break_after: false,
            },
        ];

        assert_eq!(
            probe(&metrics, 0, 25),
            LineBreakProbeV1::Selected(LineBreakDecisionV1 {
                end_index: 2,
                measured_width_emu: 20,
                reaches_metric_end: false,
            })
        );
    }

    #[test]
    fn overflow_without_fitting_break_is_explicitly_unbreakable() {
        let metrics = [
            Metric {
                advance: 10,
                break_after: false,
            },
            Metric {
                advance: 10,
                break_after: false,
            },
        ];

        assert_eq!(probe(&metrics, 0, 15), LineBreakProbeV1::Unbreakable);
    }

    #[test]
    fn probe_from_nonzero_metric_index_preserves_relative_boundary() {
        let metrics = [
            Metric {
                advance: 50,
                break_after: true,
            },
            Metric {
                advance: 7,
                break_after: false,
            },
            Metric {
                advance: 8,
                break_after: true,
            },
            Metric {
                advance: 9,
                break_after: false,
            },
        ];

        assert_eq!(
            probe(&metrics, 1, 18),
            LineBreakProbeV1::Selected(LineBreakDecisionV1 {
                end_index: 3,
                measured_width_emu: 15,
                reaches_metric_end: false,
            })
        );
    }
}
