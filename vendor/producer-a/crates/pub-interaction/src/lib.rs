//! UI-framework-neutral interaction primitives for the Publisher successor.
//!
//! This crate is deliberately downstream of `pub-model` and upstream of any
//! concrete desktop/web UI toolkit. It owns transient view/session mechanics
//! such as selection, document<->screen transforms and deterministic hit
//! testing. It does not own authoring truth and does not mutate PUB source
//! state directly.

use pub_model::{LengthEmu, NodeId, RectEmu};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentPoint {
    pub x: LengthEmu,
    pub y: LengthEmu,
}

impl DocumentPoint {
    pub const fn new(x: LengthEmu, y: LengthEmu) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTransformError {
    InvalidScale,
    NonFiniteScreenCoordinate,
    DocumentCoordinateOutOfRange,
}

impl std::fmt::Display for ViewTransformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScale => {
                formatter.write_str("pixels_per_emu must be finite and strictly positive")
            }
            Self::NonFiniteScreenCoordinate => {
                formatter.write_str("screen coordinate must be finite")
            }
            Self::DocumentCoordinateOutOfRange => {
                formatter.write_str("screen coordinate maps outside the i64 EMU range")
            }
        }
    }
}

impl std::error::Error for ViewTransformError {}

/// Transient view transform between canonical document EMU and screen pixels.
///
/// Floating point is intentionally confined to the interaction/view boundary;
/// canonical authoring geometry remains exact EMU in `pub-model`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    screen_origin: ScreenPoint,
    pixels_per_emu: f64,
}

impl ViewTransform {
    pub fn new(
        screen_origin: ScreenPoint,
        pixels_per_emu: f64,
    ) -> Result<Self, ViewTransformError> {
        if !screen_origin.x.is_finite() || !screen_origin.y.is_finite() {
            return Err(ViewTransformError::NonFiniteScreenCoordinate);
        }
        if !pixels_per_emu.is_finite() || pixels_per_emu <= 0.0 {
            return Err(ViewTransformError::InvalidScale);
        }

        Ok(Self {
            screen_origin,
            pixels_per_emu,
        })
    }

    pub const fn screen_origin(self) -> ScreenPoint {
        self.screen_origin
    }

    pub const fn pixels_per_emu(self) -> f64 {
        self.pixels_per_emu
    }

    pub fn document_to_screen(self, point: DocumentPoint) -> ScreenPoint {
        ScreenPoint {
            x: self.screen_origin.x + point.x.get() as f64 * self.pixels_per_emu,
            y: self.screen_origin.y + point.y.get() as f64 * self.pixels_per_emu,
        }
    }

    pub fn screen_to_document(
        self,
        point: ScreenPoint,
    ) -> Result<DocumentPoint, ViewTransformError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(ViewTransformError::NonFiniteScreenCoordinate);
        }

        let x = (point.x - self.screen_origin.x) / self.pixels_per_emu;
        let y = (point.y - self.screen_origin.y) / self.pixels_per_emu;

        Ok(DocumentPoint {
            x: LengthEmu::new(round_emu(x)?),
            y: LengthEmu::new(round_emu(y)?),
        })
    }
}

fn round_emu(value: f64) -> Result<i64, ViewTransformError> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(ViewTransformError::DocumentCoordinateOutOfRange);
    }
    Ok(value.round() as i64)
}

/// Transient selection state keyed by canonical authoring identity.
///
/// Selection is intentionally not persisted into `pub-model` or EditorProject.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionState {
    selected: BTreeSet<NodeId>,
    primary: Option<NodeId>,
}

impl SelectionState {
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn contains(&self, node_id: NodeId) -> bool {
        self.selected.contains(&node_id)
    }

    pub fn primary(&self) -> Option<NodeId> {
        self.primary
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.selected.iter().copied()
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.primary = None;
    }

    pub fn select_only(&mut self, node_id: NodeId) {
        self.selected.clear();
        self.selected.insert(node_id);
        self.primary = Some(node_id);
    }

    /// Toggles one node in a multi-selection.
    ///
    /// A newly added node becomes primary. Removing the primary falls back to
    /// the lowest canonical NodeId so behavior stays deterministic.
    pub fn toggle(&mut self, node_id: NodeId) {
        if self.selected.remove(&node_id) {
            if self.primary == Some(node_id) {
                self.primary = self.selected.iter().next().copied();
            }
            return;
        }

        self.selected.insert(node_id);
        self.primary = Some(node_id);
    }
}

/// One page-local, axis-aligned hit target.
///
/// `z_order` is semantic stacking order; `paint_order` is the stable
/// tie-break inside one z level. Rich path/text/transform hit testing belongs
/// to later interaction slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitTestEntry {
    pub node_id: NodeId,
    pub bounds: RectEmu,
    pub z_order: i64,
    pub paint_order: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HitTestIndex {
    entries: Vec<HitTestEntry>,
}

impl HitTestIndex {
    pub fn new(mut entries: Vec<HitTestEntry>) -> Self {
        entries.sort_by_key(|entry| (entry.z_order, entry.paint_order, entry.node_id));
        Self { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the visually topmost axis-aligned target at `point`.
    pub fn topmost_at(&self, point: DocumentPoint) -> Option<NodeId> {
        self.entries
            .iter()
            .rev()
            .find(|entry| bounds_contains(entry.bounds, point))
            .map(|entry| entry.node_id)
    }

    /// Returns all hits from topmost to bottommost.
    ///
    /// This provides the deterministic basis for Publisher-like overlap
    /// selection cycling without making cycling policy part of the hit index.
    pub fn hit_stack(&self, point: DocumentPoint) -> Vec<NodeId> {
        self.entries
            .iter()
            .rev()
            .filter(|entry| bounds_contains(entry.bounds, point))
            .map(|entry| entry.node_id)
            .collect()
    }
}

fn bounds_contains(bounds: RectEmu, point: DocumentPoint) -> bool {
    if bounds.width.get() <= 0 || bounds.height.get() <= 0 {
        return false;
    }

    let Some(right) = bounds.right() else {
        return false;
    };
    let Some(bottom) = bounds.bottom() else {
        return false;
    };

    point.x >= bounds.x && point.x <= right && point.y >= bounds.y && point.y <= bottom
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTransactionError {
    InvalidBounds,
    PointerDeltaOverflow,
    PositionOverflow,
    BoundsOverflow,
}

impl std::fmt::Display for MoveTransactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBounds => {
                formatter.write_str("move transaction requires positive non-overflowing bounds")
            }
            Self::PointerDeltaOverflow => {
                formatter.write_str("pointer movement exceeds canonical EMU delta range")
            }
            Self::PositionOverflow => {
                formatter.write_str("pointer movement moves the node origin outside EMU range")
            }
            Self::BoundsOverflow => {
                formatter.write_str("preview bounds overflow canonical EMU range")
            }
        }
    }
}

impl std::error::Error for MoveTransactionError {}

/// UI-neutral transient move gesture.
///
/// The transaction never mutates authoring state. It derives preview geometry
/// from one immutable authored rectangle plus exact document-space pointer
/// positions. A caller may commit the final x/y through its semantic editor
/// command boundary when the gesture ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveTransaction {
    node_id: NodeId,
    before: RectEmu,
    pointer_start: DocumentPoint,
    preview: RectEmu,
}

impl MoveTransaction {
    pub fn begin(
        node_id: NodeId,
        before: RectEmu,
        pointer_start: DocumentPoint,
    ) -> Result<Self, MoveTransactionError> {
        if before.width.get() <= 0
            || before.height.get() <= 0
            || before.right().is_none()
            || before.bottom().is_none()
        {
            return Err(MoveTransactionError::InvalidBounds);
        }

        Ok(Self {
            node_id,
            before,
            pointer_start,
            preview: before,
        })
    }

    pub const fn node_id(self) -> NodeId {
        self.node_id
    }

    pub const fn before_bounds(self) -> RectEmu {
        self.before
    }

    pub const fn preview_bounds(self) -> RectEmu {
        self.preview
    }

    pub const fn pointer_start(self) -> DocumentPoint {
        self.pointer_start
    }

    pub fn has_moved(self) -> bool {
        self.preview != self.before
    }

    pub fn update(&mut self, pointer: DocumentPoint) -> Result<RectEmu, MoveTransactionError> {
        let dx = pointer
            .x
            .checked_sub(self.pointer_start.x)
            .ok_or(MoveTransactionError::PointerDeltaOverflow)?;
        let dy = pointer
            .y
            .checked_sub(self.pointer_start.y)
            .ok_or(MoveTransactionError::PointerDeltaOverflow)?;
        let x = self
            .before
            .x
            .checked_add(dx)
            .ok_or(MoveTransactionError::PositionOverflow)?;
        let y = self
            .before
            .y
            .checked_add(dy)
            .ok_or(MoveTransactionError::PositionOverflow)?;
        let preview = RectEmu::new(x, y, self.before.width, self.before.height);

        if preview.right().is_none() || preview.bottom().is_none() {
            return Err(MoveTransactionError::BoundsOverflow);
        }

        self.preview = preview;
        Ok(preview)
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

    #[test]
    fn view_transform_round_trips_canonical_emu_at_view_boundary() {
        let transform =
            ViewTransform::new(ScreenPoint::new(100.0, 50.0), 0.25).expect("valid transform");
        let document = DocumentPoint::new(LengthEmu::new(-200), LengthEmu::new(400));

        let screen = transform.document_to_screen(document);
        assert_eq!(screen, ScreenPoint::new(50.0, 150.0));
        assert_eq!(
            transform.screen_to_document(screen).expect("round trip"),
            document
        );
    }

    #[test]
    fn invalid_view_scale_fails_closed() {
        assert_eq!(
            ViewTransform::new(ScreenPoint::new(0.0, 0.0), 0.0),
            Err(ViewTransformError::InvalidScale)
        );
        assert_eq!(
            ViewTransform::new(ScreenPoint::new(0.0, 0.0), f64::NAN),
            Err(ViewTransformError::InvalidScale)
        );
    }

    #[test]
    fn selection_primary_and_toggle_are_deterministic() {
        let first = node_id(1);
        let second = node_id(2);
        let mut selection = SelectionState::default();

        selection.select_only(second);
        selection.toggle(first);
        assert_eq!(selection.primary(), Some(first));
        assert_eq!(selection.iter().collect::<Vec<_>>(), vec![first, second]);

        selection.toggle(first);
        assert_eq!(selection.primary(), Some(second));
        assert_eq!(selection.iter().collect::<Vec<_>>(), vec![second]);

        selection.clear();
        assert!(selection.is_empty());
        assert_eq!(selection.primary(), None);
    }

    #[test]
    fn hit_test_returns_topmost_by_z_then_paint_order() {
        let back = node_id(1);
        let middle = node_id(2);
        let front = node_id(3);
        let index = HitTestIndex::new(vec![
            HitTestEntry {
                node_id: front,
                bounds: rect(0, 0, 100, 100),
                z_order: 10,
                paint_order: 2,
            },
            HitTestEntry {
                node_id: back,
                bounds: rect(0, 0, 100, 100),
                z_order: 5,
                paint_order: 99,
            },
            HitTestEntry {
                node_id: middle,
                bounds: rect(0, 0, 100, 100),
                z_order: 10,
                paint_order: 1,
            },
        ]);

        let point = DocumentPoint::new(LengthEmu::new(50), LengthEmu::new(50));
        assert_eq!(index.topmost_at(point), Some(front));
        assert_eq!(index.hit_stack(point), vec![front, middle, back]);
    }

    #[test]
    fn move_transaction_derives_exact_preview_without_mutating_size() {
        let node = node_id(9);
        let before = rect(100, 200, 300, 400);
        let mut transaction = MoveTransaction::begin(
            node,
            before,
            DocumentPoint::new(LengthEmu::new(10), LengthEmu::new(20)),
        )
        .expect("valid move transaction");

        assert!(!transaction.has_moved());
        assert_eq!(
            transaction
                .update(DocumentPoint::new(LengthEmu::new(60), LengthEmu::new(-10),))
                .expect("bounded move"),
            rect(150, 170, 300, 400)
        );
        assert!(transaction.has_moved());
        assert_eq!(transaction.node_id(), node);
        assert_eq!(transaction.before_bounds(), before);
        assert_eq!(transaction.preview_bounds().width, before.width);
        assert_eq!(transaction.preview_bounds().height, before.height);
    }

    #[test]
    fn move_transaction_supports_off_page_preview() {
        let mut transaction = MoveTransaction::begin(
            node_id(10),
            rect(50, 50, 100, 100),
            DocumentPoint::new(LengthEmu::new(100), LengthEmu::new(100)),
        )
        .expect("valid move transaction");

        let preview = transaction
            .update(DocumentPoint::new(
                LengthEmu::new(-100),
                LengthEmu::new(-200),
            ))
            .expect("negative authored coordinates are valid");

        assert_eq!(preview, rect(-150, -250, 100, 100));
    }

    #[test]
    fn move_transaction_fails_closed_on_invalid_or_overflowing_geometry() {
        assert_eq!(
            MoveTransaction::begin(
                node_id(11),
                rect(0, 0, 0, 100),
                DocumentPoint::new(LengthEmu::ZERO, LengthEmu::ZERO),
            ),
            Err(MoveTransactionError::InvalidBounds)
        );

        let mut transaction = MoveTransaction::begin(
            node_id(12),
            rect(0, 0, 100, 100),
            DocumentPoint::new(LengthEmu::ZERO, LengthEmu::ZERO),
        )
        .expect("valid move transaction");
        assert_eq!(
            transaction
                .update(DocumentPoint::new(
                    LengthEmu::new(i64::MAX),
                    LengthEmu::ZERO,
                ))
                .expect_err("positive width must overflow at maximum x"),
            MoveTransactionError::BoundsOverflow
        );
        assert_eq!(transaction.preview_bounds(), rect(0, 0, 100, 100));
    }

    #[test]
    fn hit_test_supports_off_page_bounds_and_rejects_invalid_rectangles() {
        let valid = node_id(1);
        let invalid = node_id(2);
        let index = HitTestIndex::new(vec![
            HitTestEntry {
                node_id: valid,
                bounds: rect(-100, -100, 80, 80),
                z_order: 0,
                paint_order: 0,
            },
            HitTestEntry {
                node_id: invalid,
                bounds: rect(-100, -100, 0, 80),
                z_order: 100,
                paint_order: 0,
            },
        ]);

        assert_eq!(
            index.topmost_at(DocumentPoint::new(LengthEmu::new(-50), LengthEmu::new(-50))),
            Some(valid)
        );
    }
}
