use pub_contents::ContentsFamily;
use pub_viewer::{
    FailureIntakeClass, ViewerDiagnosticSeverity, ViewerFidelityStatus,
    classify_failure_candidate, open_mature_0x2c,
};
use serde::Serialize;
use std::{env, fs, path::PathBuf, process::ExitCode};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResult {
    compatibility: Compatibility,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    publisher_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pages: Option<usize>,
    diagnostics_code: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    limitations: Vec<String>,
    checker_version: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Compatibility {
    Compatible,
    Partial,
    Unsupported,
    Invalid,
    Failed,
}

fn checker_version() -> String {
    env::var("CHAPTERA_CHECKER_VERSION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("chaptera-pub-check-{}", env!("CARGO_PKG_VERSION")))
}

fn bounded_message(value: &str) -> String {
    value.chars().take(900).collect::<String>()
}

fn family_label(family: Option<ContentsFamily>) -> Option<String> {
    match family {
        Some(ContentsFamily::Family0x22) => Some("Publisher 98/2000 family (0x22)".to_owned()),
        Some(ContentsFamily::Family0x2c) => Some("Publisher 2002+ family (0x2C)".to_owned()),
        None => None,
    }
}

fn open_failure_result(bytes: &[u8]) -> CheckResult {
    let classification = classify_failure_candidate(bytes);
    let publisher_family = family_label(classification.contents_family);
    let reason_codes = classification
        .reasons
        .iter()
        .take(12)
        .map(|reason| format!("{reason:?}"))
        .collect::<Vec<_>>();

    match classification.class {
        FailureIntakeClass::NotPub | FailureIntakeClass::SuspiciousPolyglot => CheckResult {
            compatibility: Compatibility::Invalid,
            summary:
                "The uploaded bytes do not look like a supported Microsoft Publisher document."
                    .to_owned(),
            publisher_family,
            pages: None,
            diagnostics_code: "pub_check.not_pub".to_owned(),
            limitations: reason_codes,
            checker_version: checker_version(),
        },
        FailureIntakeClass::PubDamaged => CheckResult {
            compatibility: Compatibility::Invalid,
            summary: "The file contains Publisher evidence, but its container or Contents data appears damaged or incomplete. A recovery workflow may still be possible."
                .to_owned(),
            publisher_family,
            pages: None,
            diagnostics_code: "pub_check.damaged".to_owned(),
            limitations: reason_codes,
            checker_version: checker_version(),
        },
        FailureIntakeClass::ArchiveWithPub => CheckResult {
            compatibility: Compatibility::Unsupported,
            summary: "The upload appears to be an archive containing a Publisher candidate rather than one directly supported PUB document."
                .to_owned(),
            publisher_family,
            pages: None,
            diagnostics_code: "pub_check.archive_with_pub".to_owned(),
            limitations: reason_codes,
            checker_version: checker_version(),
        },
        FailureIntakeClass::PubHighValue => {
            let summary = match classification.contents_family {
                Some(ContentsFamily::Family0x22) => {
                    "This is a recognized legacy Publisher 98/2000 family file. The current online checker does not yet claim Reader compatibility for this family."
                }
                Some(ContentsFamily::Family0x2c) => {
                    "This is a recognized Publisher 2002+ family file, but the current Reader pipeline could not open it reliably enough to claim compatibility."
                }
                None => {
                    "The file has strong Publisher evidence, but the current checker could not open it reliably enough to claim compatibility."
                }
            };
            CheckResult {
                compatibility: Compatibility::Unsupported,
                summary: summary.to_owned(),
                publisher_family,
                pages: None,
                diagnostics_code: "pub_check.recognized_but_unopenable".to_owned(),
                limitations: reason_codes,
                checker_version: checker_version(),
            }
        }
        FailureIntakeClass::PubPossible => CheckResult {
            compatibility: Compatibility::Unsupported,
            summary: "The file may be Publisher-related, but the current checker does not have enough evidence to classify it as working."
                .to_owned(),
            publisher_family,
            pages: None,
            diagnostics_code: "pub_check.possible_pub".to_owned(),
            limitations: reason_codes,
            checker_version: checker_version(),
        },
    }
}

fn inspect(bytes: &[u8]) -> CheckResult {
    match open_mature_0x2c(bytes) {
        Ok(document) => {
            let fidelity = document.fidelity_status();
            let limitations = document
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.severity == ViewerDiagnosticSeverity::FidelityWarning
                })
                .take(12)
                .map(|diagnostic| bounded_message(&diagnostic.message))
                .collect::<Vec<_>>();
            let publisher_family = document
                .source
                .format_version
                .as_ref()
                .map(|version| format!("{} {}", document.source.format, version))
                .or_else(|| Some(document.source.format.clone()));

            match fidelity {
                ViewerFidelityStatus::Supported => CheckResult {
                    compatibility: Compatibility::Compatible,
                    summary: "The current Chaptera Reader pipeline opened this file and did not emit a known fidelity warning inside its bounded support profile. This is not a claim of perfect Publisher rendering."
                        .to_owned(),
                    publisher_family,
                    pages: Some(document.pages.len()),
                    diagnostics_code: "pub_check.viewer_supported".to_owned(),
                    limitations,
                    checker_version: checker_version(),
                },
                ViewerFidelityStatus::Partial => CheckResult {
                    compatibility: Compatibility::Partial,
                    summary: "The current Chaptera Reader pipeline opened this file, but it reported one or more known fidelity limitations. The document is readable only within the current bounded support profile."
                        .to_owned(),
                    publisher_family,
                    pages: Some(document.pages.len()),
                    diagnostics_code: "pub_check.viewer_partial".to_owned(),
                    limitations,
                    checker_version: checker_version(),
                },
                ViewerFidelityStatus::Unsupported => CheckResult {
                    compatibility: Compatibility::Unsupported,
                    summary: "The current Chaptera Reader pipeline did not admit this file as supported."
                        .to_owned(),
                    publisher_family,
                    pages: Some(document.pages.len()),
                    diagnostics_code: "pub_check.viewer_unsupported".to_owned(),
                    limitations,
                    checker_version: checker_version(),
                },
            }
        }
        Err(_) => open_failure_result(bytes),
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("usage: chaptera-pub-check FILE")?);
    if args.next().is_some() {
        return Err("usage: chaptera-pub-check FILE".to_owned());
    }

    let bytes = fs::read(&path).map_err(|_| "checker could not read the admitted file".to_owned())?;
    let result = inspect(&bytes);
    let json = serde_json::to_string(&result)
        .map_err(|_| "checker could not serialize the compatibility receipt".to_owned())?;
    println!("{json}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let failure = CheckResult {
                compatibility: Compatibility::Failed,
                summary: "The checker could not complete this request reliably.".to_owned(),
                publisher_family: None,
                pages: None,
                diagnostics_code: "pub_check.internal_failure".to_owned(),
                limitations: vec![bounded_message(&message)],
                checker_version: checker_version(),
            };
            if let Ok(json) = serde_json::to_string(&failure) {
                println!("{json}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_invalid_not_unsupported() {
        let result = inspect(b"this is definitely ordinary text");
        assert!(matches!(result.compatibility, Compatibility::Invalid));
        assert_eq!(result.diagnostics_code, "pub_check.not_pub");
    }

    #[test]
    fn opaque_binary_never_becomes_compatible() {
        let result = inspect(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(matches!(
            result.compatibility,
            Compatibility::Unsupported | Compatibility::Invalid
        ));
    }

    #[test]
    fn user_visible_strings_are_bounded() {
        assert_eq!(bounded_message(&"x".repeat(1200)).len(), 900);
    }
}
