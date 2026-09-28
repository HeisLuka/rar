//! Native rematerialization of the closed source-neutral creation interaction laws.
//!
//! BoxDraw V1 owns exact document-space rectangle geometry only.
//! CanvasToolState V1 owns transient tool + gesture ownership only.
//! Neither layer mutates the authoring document.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_SAFE_EMU: i64 = 9_007_199_254_740_991;
pub const MIN_SAFE_EMU: i64 = -MAX_SAFE_EMU;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationInteractionError {
    pub code: &'static str,
    pub message: String,
}

impl CreationInteractionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CreationInteractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CreationInteractionError {}

fn checked_emu(value: i64, label: &str) -> Result<i64, CreationInteractionError> {
    if !(MIN_SAFE_EMU..=MAX_SAFE_EMU).contains(&value) {
        return Err(CreationInteractionError::new(
            "emu_out_of_range",
            format!("{label} outside JavaScript-safe EMU range"),
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointEmuV1 {
    pub x: i64,
    pub y: i64,
}

impl PointEmuV1 {
    pub fn new(x: i64, y: i64) -> Result<Self, CreationInteractionError> {
        Ok(Self {
            x: checked_emu(x, "point.x")?,
            y: checked_emu(y, "point.y")?,
        })
    }

    fn validate(self, label: &str) -> Result<Self, CreationInteractionError> {
        checked_emu(self.x, &format!("{label}.x"))?;
        checked_emu(self.y, &format!("{label}.y"))?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectEmuV1 {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxDrawTransactionV1 {
    pub page_id: String,
    pub anchor: PointEmuV1,
    pub current: PointEmuV1,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoxDrawPreviewStatusV1 {
    Cancelled,
    NoChange,
    Preview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxDrawPreviewV1 {
    pub status: BoxDrawPreviewStatusV1,
    pub bounds: Option<RectEmuV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoxDrawCommitStatusV1 {
    Cancelled,
    NoChange,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxDrawCommitV1 {
    pub status: BoxDrawCommitStatusV1,
    pub page_id: String,
    pub bounds: Option<RectEmuV1>,
}

fn validate_page_id(page_id: &str) -> Result<(), CreationInteractionError> {
    if page_id.is_empty() {
        return Err(CreationInteractionError::new(
            "invalid_page_id",
            "page_id is required",
        ));
    }
    Ok(())
}

fn normalized_rect(
    start: PointEmuV1,
    end: PointEmuV1,
) -> Result<Option<RectEmuV1>, CreationInteractionError> {
    let start = start.validate("start")?;
    let end = end.validate("end")?;
    let left = start.x.min(end.x);
    let top = start.y.min(end.y);
    let right = start.x.max(end.x);
    let bottom = start.y.max(end.y);
    let width = right
        .checked_sub(left)
        .ok_or_else(|| CreationInteractionError::new("emu_overflow", "bounds.width overflow"))?;
    let height = bottom
        .checked_sub(top)
        .ok_or_else(|| CreationInteractionError::new("emu_overflow", "bounds.height overflow"))?;
    checked_emu(width, "bounds.width")?;
    checked_emu(height, "bounds.height")?;
    if width == 0 || height == 0 {
        return Ok(None);
    }
    Ok(Some(RectEmuV1 {
        x: left,
        y: top,
        width,
        height,
    }))
}

pub fn start_box_draw_v1(
    page_id: impl Into<String>,
    anchor: PointEmuV1,
) -> Result<BoxDrawTransactionV1, CreationInteractionError> {
    let page_id = page_id.into();
    validate_page_id(&page_id)?;
    let anchor = anchor.validate("anchor")?;
    Ok(BoxDrawTransactionV1 {
        page_id,
        anchor,
        current: anchor,
        cancelled: false,
    })
}

pub fn update_box_draw_v1(
    transaction: &BoxDrawTransactionV1,
    current: PointEmuV1,
) -> Result<BoxDrawTransactionV1, CreationInteractionError> {
    if transaction.cancelled {
        return Err(CreationInteractionError::new(
            "box_draw_cancelled",
            "cancelled transaction cannot be updated",
        ));
    }
    validate_page_id(&transaction.page_id)?;
    transaction.anchor.validate("transaction.anchor")?;
    Ok(BoxDrawTransactionV1 {
        page_id: transaction.page_id.clone(),
        anchor: transaction.anchor,
        current: current.validate("current")?,
        cancelled: false,
    })
}

pub fn preview_box_draw_v1(
    transaction: &BoxDrawTransactionV1,
) -> Result<BoxDrawPreviewV1, CreationInteractionError> {
    validate_page_id(&transaction.page_id)?;
    if transaction.cancelled {
        return Ok(BoxDrawPreviewV1 {
            status: BoxDrawPreviewStatusV1::Cancelled,
            bounds: None,
        });
    }
    let bounds = normalized_rect(transaction.anchor, transaction.current)?;
    Ok(match bounds {
        Some(bounds) => BoxDrawPreviewV1 {
            status: BoxDrawPreviewStatusV1::Preview,
            bounds: Some(bounds),
        },
        None => BoxDrawPreviewV1 {
            status: BoxDrawPreviewStatusV1::NoChange,
            bounds: None,
        },
    })
}

pub fn cancel_box_draw_v1(
    transaction: &BoxDrawTransactionV1,
) -> Result<BoxDrawTransactionV1, CreationInteractionError> {
    validate_page_id(&transaction.page_id)?;
    transaction.anchor.validate("transaction.anchor")?;
    transaction.current.validate("transaction.current")?;
    Ok(BoxDrawTransactionV1 {
        page_id: transaction.page_id.clone(),
        anchor: transaction.anchor,
        current: transaction.current,
        cancelled: true,
    })
}

pub fn commit_box_draw_v1(
    transaction: &BoxDrawTransactionV1,
) -> Result<BoxDrawCommitV1, CreationInteractionError> {
    validate_page_id(&transaction.page_id)?;
    if transaction.cancelled {
        return Ok(BoxDrawCommitV1 {
            status: BoxDrawCommitStatusV1::Cancelled,
            page_id: transaction.page_id.clone(),
            bounds: None,
        });
    }
    let bounds = normalized_rect(transaction.anchor, transaction.current)?;
    Ok(match bounds {
        Some(bounds) => BoxDrawCommitV1 {
            status: BoxDrawCommitStatusV1::Commit,
            page_id: transaction.page_id.clone(),
            bounds: Some(bounds),
        },
        None => BoxDrawCommitV1 {
            status: BoxDrawCommitStatusV1::NoChange,
            page_id: transaction.page_id.clone(),
            bounds: None,
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CanvasToolIdV1 {
    pub namespace: String,
    pub name: String,
}

fn valid_tool_part(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    bytes.all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    })
}

impl CanvasToolIdV1 {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, CreationInteractionError> {
        let namespace = namespace.into();
        let name = name.into();
        if !valid_tool_part(&namespace) {
            return Err(CreationInteractionError::new(
                "invalid_tool_id",
                "tool namespace must be a typed lowercase identifier",
            ));
        }
        if !valid_tool_part(&name) {
            return Err(CreationInteractionError::new(
                "invalid_tool_id",
                "tool name must be a typed lowercase identifier",
            ));
        }
        Ok(Self { namespace, name })
    }
}

pub fn select_tool_v1() -> CanvasToolIdV1 {
    CanvasToolIdV1::new("chaptera", "select").expect("static tool id")
}
pub fn rectangle_create_tool_v1() -> CanvasToolIdV1 {
    CanvasToolIdV1::new("chaptera", "rectangle_create").expect("static tool id")
}
pub fn textbox_create_tool_v1() -> CanvasToolIdV1 {
    CanvasToolIdV1::new("chaptera", "textbox_create").expect("static tool id")
}
pub fn picture_create_tool_v1() -> CanvasToolIdV1 {
    CanvasToolIdV1::new("chaptera", "picture_create").expect("static tool id")
}
pub fn picture_crop_tool_v1() -> CanvasToolIdV1 {
    CanvasToolIdV1::new("chaptera", "picture_crop").expect("static tool id")
}
pub fn link_textbox_tool_v1() -> CanvasToolIdV1 {
    CanvasToolIdV1::new("chaptera", "link_textbox").expect("static tool id")
}

pub fn base_canvas_tools_v1() -> BTreeSet<CanvasToolIdV1> {
    [
        select_tool_v1(),
        rectangle_create_tool_v1(),
        textbox_create_tool_v1(),
        picture_create_tool_v1(),
        picture_crop_tool_v1(),
        link_textbox_tool_v1(),
    ]
    .into_iter()
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerGestureOwnershipV1 {
    pub tool: CanvasToolIdV1,
    pub token: String,
}

impl PointerGestureOwnershipV1 {
    fn new(
        tool: CanvasToolIdV1,
        token: impl Into<String>,
    ) -> Result<Self, CreationInteractionError> {
        let token = token.into();
        if token.is_empty() {
            return Err(CreationInteractionError::new(
                "invalid_gesture_token",
                "gesture token is required",
            ));
        }
        Ok(Self { tool, token })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasToolStateV1 {
    pub active_tool: CanvasToolIdV1,
    pub active_gesture: Option<PointerGestureOwnershipV1>,
}

impl CanvasToolStateV1 {
    pub fn validate(&self) -> Result<(), CreationInteractionError> {
        CanvasToolIdV1::new(
            self.active_tool.namespace.clone(),
            self.active_tool.name.clone(),
        )?;
        if let Some(gesture) = &self.active_gesture {
            CanvasToolIdV1::new(gesture.tool.namespace.clone(), gesture.tool.name.clone())?;
            if gesture.token.is_empty() {
                return Err(CreationInteractionError::new(
                    "invalid_gesture_token",
                    "gesture token is required",
                ));
            }
            if gesture.tool != self.active_tool {
                return Err(CreationInteractionError::new(
                    "gesture_owner_mismatch",
                    "active gesture must be owned by the active tool",
                ));
            }
        }
        Ok(())
    }
}

pub fn default_canvas_tool_state_v1() -> CanvasToolStateV1 {
    CanvasToolStateV1 {
        active_tool: select_tool_v1(),
        active_gesture: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasToolActionV1 {
    NoChange,
    ToolChanged,
    GestureStarted,
    GestureUpdated,
    GestureEnded,
    GestureCancelled,
    GestureCancelledThenToolChanged,
    DeactivatedToSelect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasToolTransitionV1 {
    pub state: CanvasToolStateV1,
    pub action: CanvasToolActionV1,
    pub cancelled_gesture_token: Option<String>,
    pub commit_requested: bool,
}

fn transition(state: CanvasToolStateV1, action: CanvasToolActionV1) -> CanvasToolTransitionV1 {
    CanvasToolTransitionV1 {
        state,
        action,
        cancelled_gesture_token: None,
        commit_requested: false,
    }
}

pub fn activate_canvas_tool_v1(
    state: &CanvasToolStateV1,
    tool: CanvasToolIdV1,
) -> Result<CanvasToolTransitionV1, CreationInteractionError> {
    state.validate()?;
    CanvasToolIdV1::new(tool.namespace.clone(), tool.name.clone())?;
    if state.active_tool == tool {
        return Ok(transition(state.clone(), CanvasToolActionV1::NoChange));
    }
    let cancelled = state
        .active_gesture
        .as_ref()
        .map(|gesture| gesture.token.clone());
    Ok(CanvasToolTransitionV1 {
        state: CanvasToolStateV1 {
            active_tool: tool,
            active_gesture: None,
        },
        action: if cancelled.is_some() {
            CanvasToolActionV1::GestureCancelledThenToolChanged
        } else {
            CanvasToolActionV1::ToolChanged
        },
        cancelled_gesture_token: cancelled,
        commit_requested: false,
    })
}

fn require_owner(
    state: &CanvasToolStateV1,
    tool: &CanvasToolIdV1,
    token: Option<&str>,
    require_gesture: bool,
) -> Result<(), CreationInteractionError> {
    state.validate()?;
    CanvasToolIdV1::new(tool.namespace.clone(), tool.name.clone())?;
    if tool != &state.active_tool {
        return Err(CreationInteractionError::new(
            "tool_owner_mismatch",
            "pointer event tool does not own the active canvas tool",
        ));
    }
    if !require_gesture {
        return Ok(());
    }
    let gesture = state.active_gesture.as_ref().ok_or_else(|| {
        CreationInteractionError::new("no_active_gesture", "no active pointer gesture")
    })?;
    if &gesture.tool != tool {
        return Err(CreationInteractionError::new(
            "tool_owner_mismatch",
            "pointer event tool does not own the active gesture",
        ));
    }
    let token = token.filter(|value| !value.is_empty()).ok_or_else(|| {
        CreationInteractionError::new(
            "gesture_token_mismatch",
            "pointer event gesture token mismatch",
        )
    })?;
    if gesture.token != token {
        return Err(CreationInteractionError::new(
            "gesture_token_mismatch",
            "pointer event gesture token mismatch",
        ));
    }
    Ok(())
}

pub fn start_pointer_gesture_v1(
    state: &CanvasToolStateV1,
    tool: CanvasToolIdV1,
    token: impl Into<String>,
) -> Result<CanvasToolTransitionV1, CreationInteractionError> {
    require_owner(state, &tool, None, false)?;
    if state.active_gesture.is_some() {
        return Err(CreationInteractionError::new(
            "gesture_already_active",
            "a pointer gesture is already active",
        ));
    }
    let gesture = PointerGestureOwnershipV1::new(tool, token)?;
    Ok(transition(
        CanvasToolStateV1 {
            active_tool: state.active_tool.clone(),
            active_gesture: Some(gesture),
        },
        CanvasToolActionV1::GestureStarted,
    ))
}

pub fn update_pointer_gesture_v1(
    state: &CanvasToolStateV1,
    tool: &CanvasToolIdV1,
    token: &str,
) -> Result<CanvasToolTransitionV1, CreationInteractionError> {
    require_owner(state, tool, Some(token), true)?;
    Ok(transition(
        state.clone(),
        CanvasToolActionV1::GestureUpdated,
    ))
}

pub fn end_pointer_gesture_v1(
    state: &CanvasToolStateV1,
    tool: &CanvasToolIdV1,
    token: &str,
) -> Result<CanvasToolTransitionV1, CreationInteractionError> {
    require_owner(state, tool, Some(token), true)?;
    Ok(transition(
        CanvasToolStateV1 {
            active_tool: state.active_tool.clone(),
            active_gesture: None,
        },
        CanvasToolActionV1::GestureEnded,
    ))
}

pub fn cancel_pointer_gesture_v1(
    state: &CanvasToolStateV1,
    tool: &CanvasToolIdV1,
    token: &str,
) -> Result<CanvasToolTransitionV1, CreationInteractionError> {
    require_owner(state, tool, Some(token), true)?;
    Ok(CanvasToolTransitionV1 {
        state: CanvasToolStateV1 {
            active_tool: state.active_tool.clone(),
            active_gesture: None,
        },
        action: CanvasToolActionV1::GestureCancelled,
        cancelled_gesture_token: Some(token.to_owned()),
        commit_requested: false,
    })
}

pub fn escape_canvas_tool_v1(
    state: &CanvasToolStateV1,
) -> Result<CanvasToolTransitionV1, CreationInteractionError> {
    state.validate()?;
    if let Some(gesture) = &state.active_gesture {
        return Ok(CanvasToolTransitionV1 {
            state: CanvasToolStateV1 {
                active_tool: state.active_tool.clone(),
                active_gesture: None,
            },
            action: CanvasToolActionV1::GestureCancelled,
            cancelled_gesture_token: Some(gesture.token.clone()),
            commit_requested: false,
        });
    }
    if state.active_tool != select_tool_v1() {
        return Ok(transition(
            default_canvas_tool_state_v1(),
            CanvasToolActionV1::DeactivatedToSelect,
        ));
    }
    Ok(transition(state.clone(), CanvasToolActionV1::NoChange))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i64, y: i64) -> PointEmuV1 {
        PointEmuV1::new(x, y).unwrap()
    }

    #[test]
    fn all_drag_directions_normalize_identically() {
        let expected = RectEmuV1 {
            x: -10,
            y: -20,
            width: 40,
            height: 60,
        };
        for (anchor, current) in [
            (p(-10, -20), p(30, 40)),
            (p(30, -20), p(-10, 40)),
            (p(-10, 40), p(30, -20)),
            (p(30, 40), p(-10, -20)),
        ] {
            let tx = start_box_draw_v1("page:1", anchor).unwrap();
            let tx = update_box_draw_v1(&tx, current).unwrap();
            assert_eq!(preview_box_draw_v1(&tx).unwrap().bounds, Some(expected));
            let result = commit_box_draw_v1(&tx).unwrap();
            assert_eq!(result.status, BoxDrawCommitStatusV1::Commit);
            assert_eq!(result.bounds, Some(expected));
        }
    }

    #[test]
    fn pointer_motion_is_transient_until_commit() {
        let tx = start_box_draw_v1("page:1", p(0, 0)).unwrap();
        assert_eq!(
            preview_box_draw_v1(&tx).unwrap().status,
            BoxDrawPreviewStatusV1::NoChange
        );
        let updated = update_box_draw_v1(&tx, p(100, 50)).unwrap();
        assert_eq!(
            preview_box_draw_v1(&updated).unwrap().status,
            BoxDrawPreviewStatusV1::Preview
        );
        assert_eq!(tx.anchor, p(0, 0));
        assert_eq!(tx.current, p(0, 0));
        assert_eq!(
            commit_box_draw_v1(&updated).unwrap().status,
            BoxDrawCommitStatusV1::Commit
        );
    }

    #[test]
    fn zero_size_cancel_identity_and_overflow_match_canonical_law() {
        for current in [p(0, 20), p(20, 0), p(0, 0)] {
            let tx = update_box_draw_v1(&start_box_draw_v1("page:1", p(0, 0)).unwrap(), current)
                .unwrap();
            let result = commit_box_draw_v1(&tx).unwrap();
            assert_eq!(result.status, BoxDrawCommitStatusV1::NoChange);
            assert!(result.bounds.is_none());
        }

        let tx = update_box_draw_v1(
            &start_box_draw_v1("customer-page:42", p(1, 2)).unwrap(),
            p(11, 22),
        )
        .unwrap();
        assert_eq!(commit_box_draw_v1(&tx).unwrap().page_id, "customer-page:42");
        let cancelled = cancel_box_draw_v1(&tx).unwrap();
        assert_eq!(
            preview_box_draw_v1(&cancelled).unwrap().status,
            BoxDrawPreviewStatusV1::Cancelled
        );
        assert_eq!(
            update_box_draw_v1(&cancelled, p(40, 50)).unwrap_err().code,
            "box_draw_cancelled"
        );

        assert_eq!(
            start_box_draw_v1("", p(0, 0)).unwrap_err().code,
            "invalid_page_id"
        );
        assert_eq!(
            PointEmuV1::new(MAX_SAFE_EMU + 1, 0).unwrap_err().code,
            "emu_out_of_range"
        );
        let wide = start_box_draw_v1("page:1", p(MIN_SAFE_EMU, 0)).unwrap();
        let wide = update_box_draw_v1(&wide, p(MAX_SAFE_EMU, 1)).unwrap();
        assert_eq!(
            commit_box_draw_v1(&wide).unwrap_err().code,
            "emu_out_of_range"
        );
    }

    #[test]
    fn base_registry_and_extension_identity_match_canonical_law() {
        let base = base_canvas_tools_v1();
        assert_eq!(base.len(), 6);
        assert!(base.contains(&select_tool_v1()));
        let extension = CanvasToolIdV1::new("plugin_vendor", "star_polygon").unwrap();
        assert!(!base.contains(&extension));
        assert_eq!(
            CanvasToolIdV1::new("Widget#12", "button").unwrap_err().code,
            "invalid_tool_id"
        );
    }

    #[test]
    fn tool_owner_token_switch_and_escape_laws_are_exact() {
        let rectangle = rectangle_create_tool_v1();
        let picture = picture_create_tool_v1();
        let state = activate_canvas_tool_v1(&default_canvas_tool_state_v1(), rectangle.clone())
            .unwrap()
            .state;
        let state = start_pointer_gesture_v1(&state, rectangle.clone(), "draw-7")
            .unwrap()
            .state;
        assert_eq!(
            update_pointer_gesture_v1(&state, &rectangle, "other")
                .unwrap_err()
                .code,
            "gesture_token_mismatch"
        );
        assert_eq!(
            update_pointer_gesture_v1(&state, &picture, "draw-7")
                .unwrap_err()
                .code,
            "tool_owner_mismatch"
        );
        let same = activate_canvas_tool_v1(&state, rectangle.clone()).unwrap();
        assert_eq!(same.action, CanvasToolActionV1::NoChange);
        assert_eq!(same.state, state);

        let switched = activate_canvas_tool_v1(&state, picture.clone()).unwrap();
        assert_eq!(
            switched.action,
            CanvasToolActionV1::GestureCancelledThenToolChanged
        );
        assert_eq!(switched.cancelled_gesture_token.as_deref(), Some("draw-7"));
        assert!(!switched.commit_requested);
        assert_eq!(switched.state.active_tool, picture);
        assert!(switched.state.active_gesture.is_none());

        let link = link_textbox_tool_v1();
        let state = activate_canvas_tool_v1(&default_canvas_tool_state_v1(), link.clone())
            .unwrap()
            .state;
        let state = start_pointer_gesture_v1(&state, link.clone(), "link-1")
            .unwrap()
            .state;
        let first = escape_canvas_tool_v1(&state).unwrap();
        assert_eq!(first.action, CanvasToolActionV1::GestureCancelled);
        assert_eq!(first.state.active_tool, link);
        let second = escape_canvas_tool_v1(&first.state).unwrap();
        assert_eq!(second.action, CanvasToolActionV1::DeactivatedToSelect);
        assert_eq!(second.state.active_tool, select_tool_v1());
        let third = escape_canvas_tool_v1(&second.state).unwrap();
        assert_eq!(third.action, CanvasToolActionV1::NoChange);
    }

    #[test]
    fn second_gesture_and_explicit_cancel_are_fail_closed() {
        let rectangle = rectangle_create_tool_v1();
        let state = activate_canvas_tool_v1(&default_canvas_tool_state_v1(), rectangle.clone())
            .unwrap()
            .state;
        let state = start_pointer_gesture_v1(&state, rectangle.clone(), "g")
            .unwrap()
            .state;
        assert_eq!(
            start_pointer_gesture_v1(&state, rectangle.clone(), "g2")
                .unwrap_err()
                .code,
            "gesture_already_active"
        );
        let cancelled = cancel_pointer_gesture_v1(&state, &rectangle, "g").unwrap();
        assert_eq!(cancelled.action, CanvasToolActionV1::GestureCancelled);
        assert!(!cancelled.commit_requested);
        assert!(cancelled.state.active_gesture.is_none());
    }

    #[test]
    fn textbox_tool_can_own_boxdraw_without_leaking_tool_semantics_into_boxdraw() {
        let tool = textbox_create_tool_v1();
        let state = activate_canvas_tool_v1(&default_canvas_tool_state_v1(), tool.clone())
            .unwrap()
            .state;
        let state = start_pointer_gesture_v1(&state, tool.clone(), "textbox-draw-1")
            .unwrap()
            .state;
        let tx = start_box_draw_v1("page:1", p(10, 20)).unwrap();
        let tx = update_box_draw_v1(&tx, p(110, 220)).unwrap();
        let preview = preview_box_draw_v1(&tx).unwrap();
        assert_eq!(preview.status, BoxDrawPreviewStatusV1::Preview);
        assert_eq!(state.active_tool, tool);
        assert_eq!(
            state
                .active_gesture
                .as_ref()
                .map(|gesture| gesture.token.as_str()),
            Some("textbox-draw-1")
        );
        let ended = end_pointer_gesture_v1(&state, &tool, "textbox-draw-1").unwrap();
        assert_eq!(ended.action, CanvasToolActionV1::GestureEnded);
        assert!(!ended.commit_requested);
        let commit = commit_box_draw_v1(&tx).unwrap();
        assert_eq!(commit.status, BoxDrawCommitStatusV1::Commit);
        assert_eq!(
            commit.bounds,
            Some(RectEmuV1 {
                x: 10,
                y: 20,
                width: 100,
                height: 200
            })
        );
    }
}
