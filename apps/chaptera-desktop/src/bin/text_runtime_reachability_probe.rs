use chaptera_desktop_shaped_flow_runtime::{
    DESKTOP_SHAPED_FLOW_RUNTIME_V1, DesktopStoryLayoutV1, ExplicitDesktopFontResourceV1,
};
use chaptera_text_caret_map_adapter::{CARET_MAP_VERSION_V1, ResolvedTextCaretMapV1};
use chaptera_text_input_adapter::{TextInputCommitV1, replace_selection_with_external_text_v1};
use chaptera_text_interaction_adapter::{SESSION_VERSION_V1, TextEditSessionV1};

fn main() {
    assert_eq!(CARET_MAP_VERSION_V1, "chaptera.resolved-text-caret-map.v1");
    assert_eq!(SESSION_VERSION_V1, "chaptera.text-edit-session.v1");
    assert_eq!(
        DESKTOP_SHAPED_FLOW_RUNTIME_V1,
        "chaptera.desktop-shaped-flow-runtime.v1"
    );

    let _ = std::mem::size_of::<ResolvedTextCaretMapV1>();
    let _ = std::mem::size_of::<TextEditSessionV1>();
    let _ = std::mem::size_of::<TextInputCommitV1>();
    let _ = std::mem::size_of::<DesktopStoryLayoutV1>();
    let _ = std::mem::size_of::<ExplicitDesktopFontResourceV1<'static>>();
    let _ = std::any::type_name_of_val(&replace_selection_with_external_text_v1);
}
