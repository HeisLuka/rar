use chaptera_text_caret_map_adapter::ResolvedTextCaretMapV1;
use chaptera_text_input_adapter::domain::{StoryEditDomainV1, StoryProvenanceV1, derive_story_edit_domain_v1};
use chaptera_text_input_adapter::ingress::normalize_external_text_v1;
use chaptera_text_input_adapter::keyboard::{KeyboardCommandV1, apply_text_keyboard_policy_v1, grapheme_boundaries_v1};
use chaptera_text_interaction_adapter::TextSelectionStateV1;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read};

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Domain {
        story_id: String,
        story_text: String,
        provenance: StoryProvenanceV1,
    },
    Ingress {
        input_text: String,
    },
    Graphemes {
        text: String,
    },
    Keyboard {
        command: KeyboardCommandV1,
        story_text: String,
        domain: StoryEditDomainV1,
        selection: Box<TextSelectionStateV1>,
        caret_map: Box<ResolvedTextCaretMapV1>,
        expected_revision_id: String,
    },
}

fn main() {
    let mut payload = String::new();
    io::stdin().read_to_string(&mut payload).expect("read stdin");
    let request: Request = serde_json::from_str(&payload).expect("parse request");
    let output: Value = match request {
        Request::Domain { story_id, story_text, provenance } => {
            match derive_story_edit_domain_v1(story_id, &story_text, provenance) {
                Ok(value) => json!({"ok": value}),
                Err(error) => json!({"error_code": error.code}),
            }
        }
        Request::Ingress { input_text } => match normalize_external_text_v1(&input_text) {
            Ok(value) => json!({"ok": value}),
            Err(_) => json!({"error_code": "text_ingress_rejected"}),
        },
        Request::Graphemes { text } => match grapheme_boundaries_v1(&text) {
            Ok(boundaries) => json!({"ok": {"boundaries": boundaries}}),
            Err(error) => json!({"error_code": error.code}),
        },
        Request::Keyboard {
            command,
            story_text,
            domain,
            selection,
            caret_map,
            expected_revision_id,
        } => match apply_text_keyboard_policy_v1(
            command,
            &story_text,
            &domain,
            &selection,
            &caret_map,
            &expected_revision_id,
        ) {
            Ok(value) => json!({"ok": value}),
            Err(error) => json!({"error_code": error.code}),
        },
    };
    println!("{}", serde_json::to_string(&output).expect("serialize output"));
}
