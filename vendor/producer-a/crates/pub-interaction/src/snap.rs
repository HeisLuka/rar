use pub_model::{LengthEmu, NodeId, RectEmu};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapAxis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapAnchorKind {
    Min,
    Center,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapTargetKind {
    PageEdge,
    PageCenter,
    ObjectEdge,
    ObjectCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapObject {
    pub node_id: NodeId,
    pub bounds: RectEmu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapFeedback {
    pub axis: SnapAxis,
    pub position: LengthEmu,
    pub moving_anchor: SnapAnchorKind,
    pub target_anchor: SnapAnchorKind,
    pub target_kind: SnapTargetKind,
    pub target_node_id: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapResult {
    pub bounds: RectEmu,
    pub x: Option<SnapFeedback>,
    pub y: Option<SnapFeedback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapError {
    InvalidPageSize,
    InvalidTolerance,
    InvalidMovingBounds,
    PositionOverflow,
    BoundsOverflow,
}

impl std::fmt::Display for SnapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPageSize => formatter.write_str("snap page size must be positive"),
            Self::InvalidTolerance => formatter.write_str("snap tolerance must be non-negative"),
            Self::InvalidMovingBounds => {
                formatter.write_str("snap moving bounds must be positive and non-overflowing")
            }
            Self::PositionOverflow => {
                formatter.write_str("snap correction moves the node origin outside EMU range")
            }
            Self::BoundsOverflow => {
                formatter.write_str("snapped bounds overflow canonical EMU range")
            }
        }
    }
}

impl std::error::Error for SnapError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapIndex {
    page_width: LengthEmu,
    page_height: LengthEmu,
    objects: Vec<SnapObject>,
}

impl SnapIndex {
    pub fn new(
        page_width: LengthEmu,
        page_height: LengthEmu,
        mut objects: Vec<SnapObject>,
    ) -> Result<Self, SnapError> {
        if page_width.get() <= 0 || page_height.get() <= 0 {
            return Err(SnapError::InvalidPageSize);
        }

        // Invalid peer rectangles are deliberately omitted instead of disabling
        // snapping for every otherwise-valid object on the page.
        objects.retain(|object| valid_bounds(object.bounds));
        objects.sort_by_key(|object| object.node_id);

        Ok(Self {
            page_width,
            page_height,
            objects,
        })
    }

    pub fn snap_rect(
        &self,
        moving_node_id: NodeId,
        bounds: RectEmu,
        tolerance: LengthEmu,
    ) -> Result<SnapResult, SnapError> {
        if tolerance.get() < 0 {
            return Err(SnapError::InvalidTolerance);
        }
        if !valid_bounds(bounds) {
            return Err(SnapError::InvalidMovingBounds);
        }

        let x_match = self.best_axis_match(
            SnapAxis::X,
            moving_node_id,
            axis_anchors(bounds, SnapAxis::X).ok_or(SnapError::BoundsOverflow)?,
            tolerance,
        );
        let y_match = self.best_axis_match(
            SnapAxis::Y,
            moving_node_id,
            axis_anchors(bounds, SnapAxis::Y).ok_or(SnapError::BoundsOverflow)?,
            tolerance,
        );

        let x = apply_correction(bounds.x, x_match.map(|candidate| candidate.correction))?;
        let y = apply_correction(bounds.y, y_match.map(|candidate| candidate.correction))?;
        let snapped = RectEmu::new(x, y, bounds.width, bounds.height);

        if snapped.right().is_none() || snapped.bottom().is_none() {
            return Err(SnapError::BoundsOverflow);
        }

        Ok(SnapResult {
            bounds: snapped,
            x: x_match.map(|candidate| candidate.feedback),
            y: y_match.map(|candidate| candidate.feedback),
        })
    }

    fn best_axis_match(
        &self,
        axis: SnapAxis,
        moving_node_id: NodeId,
        moving_anchors: [(SnapAnchorKind, LengthEmu); 3],
        tolerance: LengthEmu,
    ) -> Option<SnapCandidate> {
        let mut targets = self.page_targets(axis);
        for object in &self.objects {
            if object.node_id == moving_node_id {
                continue;
            }
            let Some(anchors) = axis_anchors(object.bounds, axis) else {
                continue;
            };
            for (anchor, position) in anchors {
                targets.push(SnapTarget {
                    position,
                    anchor,
                    kind: if anchor == SnapAnchorKind::Center {
                        SnapTargetKind::ObjectCenter
                    } else {
                        SnapTargetKind::ObjectEdge
                    },
                    node_id: Some(object.node_id),
                });
            }
        }

        let tolerance = tolerance.get().unsigned_abs();
        let mut best = None;

        for (moving_anchor, moving_position) in moving_anchors {
            for target in &targets {
                let Some(correction) = target.position.get().checked_sub(moving_position.get())
                else {
                    continue;
                };
                let distance = correction.unsigned_abs();
                if distance > tolerance {
                    continue;
                }

                let candidate = SnapCandidate {
                    correction,
                    distance,
                    feedback: SnapFeedback {
                        axis,
                        position: target.position,
                        moving_anchor,
                        target_anchor: target.anchor,
                        target_kind: target.kind,
                        target_node_id: target.node_id,
                    },
                };

                if best
                    .as_ref()
                    .is_none_or(|current: &SnapCandidate| candidate.key() < current.key())
                {
                    best = Some(candidate);
                }
            }
        }

        best
    }

    fn page_targets(&self, axis: SnapAxis) -> Vec<SnapTarget> {
        let extent = match axis {
            SnapAxis::X => self.page_width,
            SnapAxis::Y => self.page_height,
        };
        let center = LengthEmu::new(extent.get() / 2);

        vec![
            SnapTarget {
                position: LengthEmu::ZERO,
                anchor: SnapAnchorKind::Min,
                kind: SnapTargetKind::PageEdge,
                node_id: None,
            },
            SnapTarget {
                position: center,
                anchor: SnapAnchorKind::Center,
                kind: SnapTargetKind::PageCenter,
                node_id: None,
            },
            SnapTarget {
                position: extent,
                anchor: SnapAnchorKind::Max,
                kind: SnapTargetKind::PageEdge,
                node_id: None,
            },
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapTarget {
    position: LengthEmu,
    anchor: SnapAnchorKind,
    kind: SnapTargetKind,
    node_id: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapCandidate {
    correction: i64,
    distance: u64,
    feedback: SnapFeedback,
}

impl SnapCandidate {
    fn key(
        self,
    ) -> (
        u64,
        SnapTargetKind,
        Option<NodeId>,
        SnapAnchorKind,
        SnapAnchorKind,
    ) {
        (
            self.distance,
            self.feedback.target_kind,
            self.feedback.target_node_id,
            self.feedback.target_anchor,
            self.feedback.moving_anchor,
        )
    }
}

fn valid_bounds(bounds: RectEmu) -> bool {
    bounds.width.get() > 0
        && bounds.height.get() > 0
        && bounds.right().is_some()
        && bounds.bottom().is_some()
}

fn axis_anchors(bounds: RectEmu, axis: SnapAxis) -> Option<[(SnapAnchorKind, LengthEmu); 3]> {
    let (min, extent, max) = match axis {
        SnapAxis::X => (bounds.x, bounds.width, bounds.right()?),
        SnapAxis::Y => (bounds.y, bounds.height, bounds.bottom()?),
    };
    let center = min.checked_add(LengthEmu::new(extent.get() / 2))?;

    Some([
        (SnapAnchorKind::Min, min),
        (SnapAnchorKind::Center, center),
        (SnapAnchorKind::Max, max),
    ])
}

fn apply_correction(origin: LengthEmu, correction: Option<i64>) -> Result<LengthEmu, SnapError> {
    match correction {
        Some(correction) => origin
            .checked_add(LengthEmu::new(correction))
            .ok_or(SnapError::PositionOverflow),
        None => Ok(origin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::CanonicalId;

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    fn rect(x: i64, y: i64, width: i64, height: i64) -> RectEmu {
        RectEmu::new(
            LengthEmu::new(x),
            LengthEmu::new(y),
            LengthEmu::new(width),
            LengthEmu::new(height),
        )
    }

    fn index(objects: Vec<SnapObject>) -> SnapIndex {
        SnapIndex::new(LengthEmu::new(1_000), LengthEmu::new(800), objects).expect("valid page")
    }

    #[test]
    fn snaps_nearest_moving_edge_to_page_edge() {
        let result = index(Vec::new())
            .snap_rect(node_id(1), rect(6, 100, 100, 80), LengthEmu::new(10))
            .expect("snap");

        assert_eq!(result.bounds, rect(0, 100, 100, 80));
        assert_eq!(
            result.x,
            Some(SnapFeedback {
                axis: SnapAxis::X,
                position: LengthEmu::ZERO,
                moving_anchor: SnapAnchorKind::Min,
                target_anchor: SnapAnchorKind::Min,
                target_kind: SnapTargetKind::PageEdge,
                target_node_id: None,
            })
        );
        assert_eq!(result.y, None);
    }

    #[test]
    fn snaps_object_centers_without_changing_size() {
        let peer = SnapObject {
            node_id: node_id(2),
            bounds: rect(380, 200, 200, 100),
        };
        let result = index(vec![peer])
            .snap_rect(node_id(1), rect(432, 350, 100, 60), LengthEmu::new(5))
            .expect("snap");

        assert_eq!(result.bounds, rect(430, 350, 100, 60));
        assert_eq!(result.bounds.width, LengthEmu::new(100));
        assert_eq!(result.bounds.height, LengthEmu::new(60));
        assert_eq!(
            result.x.expect("x snap").target_kind,
            SnapTargetKind::ObjectCenter
        );
        assert_eq!(result.x.expect("x snap").target_node_id, Some(node_id(2)));
    }

    #[test]
    fn moving_node_is_excluded_from_peer_targets() {
        let moving = SnapObject {
            node_id: node_id(1),
            bounds: rect(100, 100, 100, 100),
        };
        let result = index(vec![moving])
            .snap_rect(node_id(1), rect(103, 103, 100, 100), LengthEmu::new(5))
            .expect("snap");

        assert_eq!(result.bounds, rect(103, 103, 100, 100));
        assert_eq!(result.x, None);
        assert_eq!(result.y, None);
    }

    #[test]
    fn chooses_smallest_absolute_correction_before_target_class() {
        let peer = SnapObject {
            node_id: node_id(2),
            bounds: rect(205, 300, 100, 100),
        };
        let result = index(vec![peer])
            .snap_rect(node_id(1), rect(101, 100, 100, 100), LengthEmu::new(5))
            .expect("snap");

        // Right edge 201 -> peer left 205 is +4, while left 101 -> page left
        // is outside tolerance. The object target wins by exact distance.
        assert_eq!(result.bounds.x, LengthEmu::new(105));
        assert_eq!(
            result.x.expect("x snap").target_kind,
            SnapTargetKind::ObjectEdge
        );
    }

    #[test]
    fn deterministic_tie_prefers_page_edge_before_object_edge() {
        let peer = SnapObject {
            node_id: node_id(2),
            bounds: rect(10, 300, 100, 100),
        };
        let result = index(vec![peer])
            .snap_rect(node_id(1), rect(5, 100, 100, 100), LengthEmu::new(5))
            .expect("snap");

        assert_eq!(result.bounds.x, LengthEmu::ZERO);
        assert_eq!(
            result.x.expect("x snap").target_kind,
            SnapTargetKind::PageEdge
        );
    }

    #[test]
    fn snaps_both_axes_independently() {
        let result = index(Vec::new())
            .snap_rect(node_id(1), rect(448, 347, 100, 100), LengthEmu::new(5))
            .expect("snap");

        assert_eq!(result.bounds, rect(450, 350, 100, 100));
        assert_eq!(
            result.x.expect("x snap").target_kind,
            SnapTargetKind::PageCenter
        );
        assert_eq!(
            result.y.expect("y snap").target_kind,
            SnapTargetKind::PageCenter
        );
    }

    #[test]
    fn outside_tolerance_does_not_snap() {
        let result = index(Vec::new())
            .snap_rect(node_id(1), rect(11, 111, 100, 100), LengthEmu::new(10))
            .expect("snap");

        assert_eq!(result.bounds, rect(11, 111, 100, 100));
        assert_eq!(result.x, None);
        assert_eq!(result.y, None);
    }

    #[test]
    fn invalid_peer_bounds_are_ignored_but_invalid_moving_bounds_fail() {
        let invalid_peer = SnapObject {
            node_id: node_id(2),
            bounds: rect(0, 0, 0, 100),
        };
        let index = index(vec![invalid_peer]);
        assert_eq!(
            index
                .snap_rect(node_id(1), rect(0, 0, 0, 100), LengthEmu::new(5))
                .expect_err("invalid moving bounds"),
            SnapError::InvalidMovingBounds
        );
    }

    #[test]
    fn negative_tolerance_is_rejected() {
        assert_eq!(
            index(Vec::new())
                .snap_rect(node_id(1), rect(100, 100, 100, 100), LengthEmu::new(-1))
                .expect_err("negative tolerance"),
            SnapError::InvalidTolerance
        );
    }
}
