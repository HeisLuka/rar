use chaptera_text_caret_map_adapter::{
    CaretMapBuildInputV1, ResolvedClusterV1, ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
};
use chaptera_text_interaction_adapter::{
    StoryEditDomainV1, TextEntryCandidateV1, TextInitialPositionV1, TextPointerTargetV1,
    activate_explicit_edit_text_v1, activate_pointer_text_v1, enter_text_edit_session_v1,
    exit_desktop_text_mode_v1, handoff_same_story_frame_v1, switch_text_edit_session_v1,
};
use serde_json::{Value, json};

fn cluster(start: u32, end: u32, x0: i64, x1: i64) -> ResolvedClusterV1 {
    ResolvedClusterV1 {
        start_scalar: start,
        end_scalar: end,
        page_x_start_emu: x0,
        page_x_end_emu: x1,
        frame_x_start_emu: x0,
        frame_x_end_emu: x1,
        painted: true,
        internal_caret_stops: Vec::new(),
    }
}

fn line(
    story: &str,
    id: &str,
    ordinal: u32,
    frame: &str,
    prev: Option<&str>,
    next: Option<&str>,
    y: i64,
    clusters: Vec<ResolvedClusterV1>,
) -> ResolvedLineFragmentV1 {
    ResolvedLineFragmentV1 {
        story_id: story.to_owned(),
        page_id: "page:1".to_owned(),
        frame_id: frame.to_owned(),
        line_id: id.to_owned(),
        flow_ordinal: ordinal,
        previous_line_id: prev.map(str::to_owned),
        next_line_id: next.map(str::to_owned),
        page_y_top_emu: y,
        page_y_bottom_emu: y + 20,
        frame_y_top_emu: y,
        frame_y_bottom_emu: y + 20,
        clusters,
    }
}

fn context(
    story: &str,
    len: u32,
    linked: bool,
) -> (
    StoryEditDomainV1,
    chaptera_text_caret_map_adapter::ResolvedTextCaretMapV1,
) {
    let lines = if len == 0 {
        Vec::new()
    } else if linked && len >= 2 {
        vec![
            line(
                story,
                "l1",
                0,
                "frame:A",
                None,
                Some("l2"),
                0,
                vec![cluster(0, 1, 0, 10)],
            ),
            line(
                story,
                "l2",
                1,
                "frame:B",
                Some("l1"),
                None,
                30,
                (1..len)
                    .map(|i| cluster(i, i + 1, ((i - 1) * 10) as i64, (i * 10) as i64))
                    .collect(),
            ),
        ]
    } else {
        vec![line(
            story,
            "l1",
            0,
            "frame:A",
            None,
            None,
            0,
            (0..len)
                .map(|i| cluster(i, i + 1, (i * 10) as i64, ((i + 1) * 10) as i64))
                .collect(),
        )]
    };
    let map = build_resolved_text_caret_map_v1(CaretMapBuildInputV1 {
        layout_revision_id: "layout:1".to_owned(),
        story_id: story.to_owned(),
        story_scalar_len: len,
        lines,
    })
    .unwrap();
    (
        StoryEditDomainV1 {
            story_id: story.to_owned(),
            raw_scalar_len: len,
            status: "known".to_owned(),
            caret_start_boundary: Some(0),
            caret_end_boundary: Some(len),
            domain_id: format!("domain:{story}:{len}"),
        },
        map,
    )
}

fn candidate(story: &str, frame: &str) -> TextEntryCandidateV1 {
    TextEntryCandidateV1 {
        target_id: frame.to_owned(),
        story_id: story.to_owned(),
        frame_id: Some(frame.to_owned()),
        capability: "editable".to_owned(),
        reason: None,
    }
}

fn session_summary(
    status: &str,
    session: Option<&chaptera_text_interaction_adapter::TextEditSessionV1>,
    mutation_count: u32,
    undo: bool,
) -> Value {
    json!({
        "status": status,
        "story_id": session.map(|s| s.story_id.clone()),
        "frame_id": session.and_then(|s| s.current_frame_id.clone()),
        "focus_scalar": session.map(|s| s.selection.focus_scalar),
        "session_id": session.map(|s| s.session_id.clone()),
        "incarnation": session.map(|s| s.incarnation),
        "mutation_count": mutation_count,
        "undo_group_boundary": undo,
    })
}

fn run(name: &str) -> Value {
    match name {
        "pointer_enter" => {
            let (domain, map) = context("story:1", 3, false);
            let tr = enter_text_edit_session_v1(
                "session:1",
                0,
                "doc:1",
                &candidate("story:1", "frame:A"),
                "rev:1",
                &domain,
                &map,
                "layout:1",
                Some(&TextPointerTargetV1 {
                    page_id: "page:1".into(),
                    page_x_emu: 19,
                    page_y_emu: 10,
                }),
                None,
                Vec::new(),
            )
            .unwrap();
            session_summary(
                &tr.kind,
                Some(&tr.session),
                tr.lifecycle_document_mutation_count,
                tr.undo_group_boundary,
            )
        }
        "empty_enter" => {
            let (domain, map) = context("story:new", 0, false);
            let tr = enter_text_edit_session_v1(
                "session:new",
                0,
                "doc:1",
                &candidate("story:new", "frame:new"),
                "rev:1",
                &domain,
                &map,
                "layout:1",
                None,
                Some(&TextInitialPositionV1 {
                    scalar_boundary: 0,
                    visual_stop_id: None,
                }),
                Vec::new(),
            )
            .unwrap();
            session_summary(
                &tr.kind,
                Some(&tr.session),
                tr.lifecycle_document_mutation_count,
                tr.undo_group_boundary,
            )
        }
        "linked_handoff" => {
            let (domain, map) = context("story:1", 2, true);
            let active = enter_text_edit_session_v1(
                "session:1",
                4,
                "doc:1",
                &candidate("story:1", "frame:A"),
                "rev:1",
                &domain,
                &map,
                "layout:1",
                Some(&TextPointerTargetV1 {
                    page_id: "page:1".into(),
                    page_x_emu: 0,
                    page_y_emu: 10,
                }),
                None,
                Vec::new(),
            )
            .unwrap()
            .session;
            let tr = handoff_same_story_frame_v1(
                &active,
                &candidate("story:1", "frame:B"),
                &domain,
                &map,
                "layout:1",
                Some(&TextPointerTargetV1 {
                    page_id: "page:1".into(),
                    page_x_emu: 10,
                    page_y_emu: 40,
                }),
                None,
            )
            .unwrap();
            session_summary(
                &tr.kind,
                Some(&tr.session),
                tr.lifecycle_document_mutation_count,
                tr.undo_group_boundary,
            )
        }
        "story_switch" => {
            let (d1, m1) = context("story:1", 1, false);
            let active = enter_text_edit_session_v1(
                "session:1",
                2,
                "doc:1",
                &candidate("story:1", "frame:A"),
                "rev:1",
                &d1,
                &m1,
                "layout:1",
                None,
                Some(&TextInitialPositionV1 {
                    scalar_boundary: 1,
                    visual_stop_id: None,
                }),
                Vec::new(),
            )
            .unwrap()
            .session;
            let (d2, m2) = context("story:2", 1, false);
            let tr = switch_text_edit_session_v1(
                &active,
                &candidate("story:2", "frame:A"),
                "rev:2",
                &d2,
                &m2,
                "layout:1",
                None,
                Some(&TextInitialPositionV1 {
                    scalar_boundary: 0,
                    visual_stop_id: None,
                }),
                None,
                Vec::new(),
            )
            .unwrap();
            session_summary(
                &tr.kind,
                Some(&tr.session),
                tr.lifecycle_document_mutation_count,
                tr.undo_group_boundary,
            )
        }
        "inactive_click" => {
            let (domain, map) = context("story:1", 3, false);
            let r = activate_pointer_text_v1(
                &candidate("story:1", "frame:A"),
                "rev:1",
                &domain,
                &map,
                "layout:1",
                &TextPointerTargetV1 {
                    page_id: "page:1".into(),
                    page_x_emu: 19,
                    page_y_emu: 10,
                },
                None,
                false,
                1,
                "session:1",
                "doc:1",
                None,
            )
            .unwrap();
            session_summary(
                &r.status,
                r.active_session.as_ref(),
                r.document_mutation_count,
                false,
            )
        }
        "pointer_mismatch" => {
            let (domain, map) = context("story:1", 1, false);
            let r = activate_pointer_text_v1(
                &candidate("story:1", "frame:B"),
                "rev:1",
                &domain,
                &map,
                "layout:1",
                &TextPointerTargetV1 {
                    page_id: "page:1".into(),
                    page_x_emu: 5,
                    page_y_emu: 200,
                },
                None,
                true,
                1,
                "session:1",
                "doc:1",
                None,
            )
            .unwrap();
            session_summary(
                &r.status,
                r.active_session.as_ref(),
                r.document_mutation_count,
                false,
            )
        }
        "escape_focus_fence" | "escape_exit" => {
            let (domain, map) = context("story:1", 1, false);
            let active = activate_explicit_edit_text_v1(
                "session:1",
                "doc:1",
                "rev:1",
                &candidate("story:1", "frame:A"),
                &domain,
                &map,
                "layout:1",
                None,
                None,
            )
            .unwrap()
            .active_session
            .unwrap();
            let focus = if name == "escape_focus_fence" {
                "find_replace"
            } else {
                "story_text"
            };
            let r = exit_desktop_text_mode_v1(
                Some(&active),
                "escape",
                focus,
                None,
                &["op:already-submitted".to_owned()],
                false,
            )
            .unwrap();
            let undo = r
                .exit_receipt
                .as_ref()
                .is_some_and(|x| x.undo_group_boundary);
            session_summary(
                &r.status,
                r.active_session.as_ref(),
                r.document_mutation_count,
                undo,
            )
        }
        _ => panic!("unknown scenario"),
    }
}

fn main() {
    let name = std::env::args().nth(1).expect("scenario");
    println!("{}", serde_json::to_string(&run(&name)).unwrap());
}
