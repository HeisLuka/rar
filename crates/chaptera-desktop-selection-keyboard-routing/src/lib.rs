//! Collision-safe native routing decisions for restoring ordinary Desktop keyboard behavior.
//! This crate does not mutate EditorSession or own selection/text truth.

use chaptera_canvas_creation_interaction::{
    escape_canvas_tool_v1, CanvasToolActionV1, CanvasToolStateV1, CanvasToolTransitionV1,
    CreationInteractionError,
};
use serde::{Deserialize, Serialize};

pub const BASE_NUDGE_EMU: i64 = 118_872;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusOwnerV1 {
    Canvas,
    StoryText,
    HostControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowDirectionV1 {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KeyModifiersV1 {
    pub shift: bool,
    pub control_or_command: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowRouteV1 {
    RouteStory,
    MoveObject,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrowDecisionV1 {
    pub route: ArrowRouteV1,
    pub dx_emu: i64,
    pub dy_emu: i64,
}

fn delta(direction: ArrowDirectionV1) -> (i64, i64) {
    match direction {
        ArrowDirectionV1::Left => (-BASE_NUDGE_EMU, 0),
        ArrowDirectionV1::Right => (BASE_NUDGE_EMU, 0),
        ArrowDirectionV1::Up => (0, -BASE_NUDGE_EMU),
        ArrowDirectionV1::Down => (0, BASE_NUDGE_EMU),
    }
}

/// Exact current restoration law:
/// - ordinary Story arrows remain text-owned;
/// - Alt+Arrow may escape Story focus only when the active Story owns the one
///   selected movable text object;
/// - canvas arrows admit exactly one movable selected object;
/// - Shift/Ctrl/Cmd do not silently become object-nudge modifiers.
pub fn route_arrow_v1(
    focus: FocusOwnerV1,
    direction: ArrowDirectionV1,
    modifiers: KeyModifiersV1,
    selected_count: usize,
    selected_object_movable: bool,
    active_story_owns_selected_object: bool,
) -> ArrowDecisionV1 {
    if focus == FocusOwnerV1::StoryText && !modifiers.alt {
        return ArrowDecisionV1 { route: ArrowRouteV1::RouteStory, dx_emu: 0, dy_emu: 0 };
    }
    if modifiers.shift || modifiers.control_or_command {
        return ArrowDecisionV1 { route: ArrowRouteV1::Ignored, dx_emu: 0, dy_emu: 0 };
    }

    let admitted = match focus {
        FocusOwnerV1::Canvas => {
            !modifiers.alt && selected_count == 1 && selected_object_movable
        }
        FocusOwnerV1::StoryText => {
            modifiers.alt
                && selected_count == 1
                && selected_object_movable
                && active_story_owns_selected_object
        }
        FocusOwnerV1::HostControl => false,
    };
    if !admitted {
        return ArrowDecisionV1 { route: ArrowRouteV1::Ignored, dx_emu: 0, dy_emu: 0 };
    }
    let (dx_emu, dy_emu) = delta(direction);
    ArrowDecisionV1 { route: ArrowRouteV1::MoveObject, dx_emu, dy_emu }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectAllCandidateV1 {
    pub instance_id: String,
    pub page_id: String,
    pub authored_direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectAllRouteV1 {
    RouteStory,
    SelectObjects,
    Suppressed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectAllDecisionV1 {
    pub route: SelectAllRouteV1,
    pub selected_instance_ids: Vec<String>,
    pub excluded_non_authored_count: usize,
}

pub fn route_select_all_v1(
    focus: FocusOwnerV1,
    current_page_id: &str,
    canvas_tool_is_select: bool,
    canvas_gesture_active: bool,
    candidates: &[SelectAllCandidateV1],
) -> SelectAllDecisionV1 {
    if focus == FocusOwnerV1::StoryText {
        return SelectAllDecisionV1 {
            route: SelectAllRouteV1::RouteStory,
            selected_instance_ids: Vec::new(),
            excluded_non_authored_count: 0,
        };
    }
    if focus != FocusOwnerV1::Canvas || !canvas_tool_is_select || canvas_gesture_active {
        return SelectAllDecisionV1 {
            route: SelectAllRouteV1::Suppressed,
            selected_instance_ids: Vec::new(),
            excluded_non_authored_count: 0,
        };
    }

    let mut selected = Vec::new();
    let mut excluded = 0;
    for candidate in candidates.iter().filter(|item| item.page_id == current_page_id) {
        if candidate.authored_direct {
            selected.push(candidate.instance_id.clone());
        } else {
            excluded += 1;
        }
    }
    selected.sort();
    selected.dedup();
    SelectAllDecisionV1 {
        route: SelectAllRouteV1::SelectObjects,
        selected_instance_ids: selected,
        excluded_non_authored_count: excluded,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeRouteV1 {
    HostConsumed,
    ExitTextSession,
    CanvasTransition(CanvasToolTransitionV1),
    ClearTopLevelSelection,
    NoOp,
}

pub fn route_escape_v1(
    focus: FocusOwnerV1,
    text_session_active: bool,
    canvas: &CanvasToolStateV1,
    top_level_selection_nonempty: bool,
) -> Result<EscapeRouteV1, CreationInteractionError> {
    if focus == FocusOwnerV1::HostControl {
        return Ok(EscapeRouteV1::HostConsumed);
    }
    if text_session_active {
        return Ok(EscapeRouteV1::ExitTextSession);
    }
    let transition = escape_canvas_tool_v1(canvas)?;
    if transition.action != CanvasToolActionV1::NoChange {
        return Ok(EscapeRouteV1::CanvasTransition(transition));
    }
    if top_level_selection_nonempty {
        return Ok(EscapeRouteV1::ClearTopLevelSelection);
    }
    Ok(EscapeRouteV1::NoOp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaptera_canvas_creation_interaction::{
        activate_canvas_tool_v1, default_canvas_tool_state_v1, start_pointer_gesture_v1,
        textbox_create_tool_v1,
    };

    #[test]
    fn story_arrows_stay_text_owned_but_alt_arrow_nudges_owner() {
        let ordinary = route_arrow_v1(
            FocusOwnerV1::StoryText,
            ArrowDirectionV1::Left,
            KeyModifiersV1::default(),
            1,
            true,
            true,
        );
        assert_eq!(ordinary.route, ArrowRouteV1::RouteStory);

        let alt = route_arrow_v1(
            FocusOwnerV1::StoryText,
            ArrowDirectionV1::Right,
            KeyModifiersV1 { alt: true, ..Default::default() },
            1,
            true,
            true,
        );
        assert_eq!(alt.route, ArrowRouteV1::MoveObject);
        assert_eq!((alt.dx_emu, alt.dy_emu), (BASE_NUDGE_EMU, 0));
    }

    #[test]
    fn canvas_nudge_requires_one_movable_selection() {
        let decision = route_arrow_v1(
            FocusOwnerV1::Canvas,
            ArrowDirectionV1::Up,
            KeyModifiersV1::default(),
            1,
            true,
            false,
        );
        assert_eq!(decision.route, ArrowRouteV1::MoveObject);
        assert_eq!(decision.dy_emu, -BASE_NUDGE_EMU);
        assert_eq!(
            route_arrow_v1(
                FocusOwnerV1::Canvas,
                ArrowDirectionV1::Up,
                KeyModifiersV1::default(),
                2,
                true,
                false,
            ).route,
            ArrowRouteV1::Ignored
        );
    }

    #[test]
    fn select_all_routes_by_focus_and_semantic_page_ownership() {
        let candidates = vec![
            SelectAllCandidateV1 { instance_id: "b".into(), page_id: "p1".into(), authored_direct: true },
            SelectAllCandidateV1 { instance_id: "a".into(), page_id: "p1".into(), authored_direct: true },
            SelectAllCandidateV1 { instance_id: "projection".into(), page_id: "p1".into(), authored_direct: false },
            SelectAllCandidateV1 { instance_id: "other-page".into(), page_id: "p2".into(), authored_direct: true },
        ];
        let result = route_select_all_v1(FocusOwnerV1::Canvas, "p1", true, false, &candidates);
        assert_eq!(result.route, SelectAllRouteV1::SelectObjects);
        assert_eq!(result.selected_instance_ids, vec!["a", "b"]);
        assert_eq!(result.excluded_non_authored_count, 1);
        assert_eq!(
            route_select_all_v1(FocusOwnerV1::StoryText, "p1", true, false, &candidates).route,
            SelectAllRouteV1::RouteStory
        );
    }

    #[test]
    fn escape_precedence_reuses_canvas_tool_state() {
        let select = default_canvas_tool_state_v1();
        assert!(matches!(
            route_escape_v1(FocusOwnerV1::StoryText, true, &select, true).unwrap(),
            EscapeRouteV1::ExitTextSession
        ));

        let text_tool = activate_canvas_tool_v1(&select, textbox_create_tool_v1()).unwrap().state;
        let gesture = start_pointer_gesture_v1(&text_tool, textbox_create_tool_v1(), "g1").unwrap().state;
        assert!(matches!(
            route_escape_v1(FocusOwnerV1::Canvas, false, &gesture, true).unwrap(),
            EscapeRouteV1::CanvasTransition(_)
        ));
        assert!(matches!(
            route_escape_v1(FocusOwnerV1::Canvas, false, &select, true).unwrap(),
            EscapeRouteV1::ClearTopLevelSelection
        ));
    }
}
