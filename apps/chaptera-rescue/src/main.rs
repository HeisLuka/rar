use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const APP_TITLE: &str = "Chaptera Rescue — Technical Preview";
const PRODUCT_ID: &str = "chaptera.rescue";
const PRODUCT_RECEIPT_VERSION: &str = "chaptera.rescue-product-validation.v1";

#[derive(Debug, Clone, Deserialize)]
struct FixtureIdentity {
    kind: String,
    source_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PrivacyState {
    source_free: bool,
    private_content_serialized: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductValidationReceipt {
    receipt_version: String,
    product: String,
    producer_receipt_sha256: String,
    fixture: FixtureIdentity,
    outcome: String,
    source_immutable: bool,
    artifact_count: usize,
    known_loss: bool,
    native_pub_delivery_allowed: bool,
    fail_closed: bool,
    privacy: PrivacyState,
}

#[derive(Debug, Clone, Serialize)]
struct RescueDesktopReport {
    schema_version: &'static str,
    product_id: &'static str,
    source_sha256: String,
    source_immutable: bool,
    outcome: String,
    artifact_count: usize,
    known_loss: bool,
    native_pub_delivery_allowed: bool,
    producer_receipt_sha256: String,
    note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct SelfCheck {
    schema_version: &'static str,
    product_id: &'static str,
    canonical_executable: &'static str,
    editor_ui_present: bool,
    migration_batch_ui_present: bool,
    recovery_execution_embedded: bool,
}

#[derive(Default)]
struct RescueApp {
    source_path: String,
    source_sha256: Option<String>,
    source_status: Option<String>,
    receipt_path: String,
    receipt: Option<ProductValidationReceipt>,
    receipt_status: Option<String>,
    report_path: String,
    report_status: Option<String>,
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_product_receipt(
    receipt: &ProductValidationReceipt,
    source_sha256: &str,
) -> Result<(), String> {
    if receipt.receipt_version != PRODUCT_RECEIPT_VERSION {
        return Err(format!(
            "unsupported Rescue receipt version: {}",
            receipt.receipt_version
        ));
    }
    if receipt.product != "Chaptera Rescue" {
        return Err("receipt does not identify Chaptera Rescue".to_owned());
    }
    if receipt.fixture.source_sha256 != source_sha256 {
        return Err("receipt source identity does not match the selected PUB".to_owned());
    }
    if !matches!(
        receipt.fixture.kind.as_str(),
        "natural_partial_cfb" | "healthy_control" | "partial_control"
    ) {
        return Err("receipt fixture kind is outside the current Rescue contract".to_owned());
    }
    if !matches!(
        receipt.outcome.as_str(),
        "bounded_recovered"
            | "partially_recovered"
            | "manual_review"
            | "unsupported/no_safe_recovery"
    ) {
        return Err("receipt outcome is outside the current Rescue contract".to_owned());
    }
    if !receipt.source_immutable || !receipt.fail_closed {
        return Err("receipt does not prove immutable-source fail-closed handling".to_owned());
    }
    if !receipt.privacy.source_free || receipt.privacy.private_content_serialized {
        return Err("receipt violates the source-free public product boundary".to_owned());
    }
    if !is_lower_sha256(&receipt.producer_receipt_sha256) {
        return Err("producer_receipt_sha256 is invalid".to_owned());
    }
    Ok(())
}

fn load_product_receipt(
    path: &Path,
    source_sha256: &str,
) -> Result<ProductValidationReceipt, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let receipt: ProductValidationReceipt =
        serde_json::from_slice(&bytes).map_err(|error| format!("parse Rescue receipt: {error}"))?;
    validate_product_receipt(&receipt, source_sha256)?;
    Ok(receipt)
}

fn build_report(receipt: &ProductValidationReceipt, source_sha256: &str) -> RescueDesktopReport {
    RescueDesktopReport {
        schema_version: "chaptera.rescue-desktop-report.v1",
        product_id: PRODUCT_ID,
        source_sha256: source_sha256.to_owned(),
        source_immutable: true,
        outcome: receipt.outcome.clone(),
        artifact_count: receipt.artifact_count,
        known_loss: receipt.known_loss,
        native_pub_delivery_allowed: receipt.native_pub_delivery_allowed,
        producer_receipt_sha256: receipt.producer_receipt_sha256.clone(),
        note: "This report consumes a validated Rescue product receipt; it does not claim that this GUI executed recovery.",
    }
}

fn write_report(
    source_path: &Path,
    output_path: &Path,
    receipt: &ProductValidationReceipt,
    expected_source_sha256: &str,
) -> Result<(), String> {
    if source_path == output_path {
        return Err("report output must not overwrite the source PUB".to_owned());
    }
    let before = sha256_file(source_path)?;
    if before != expected_source_sha256 {
        return Err("source PUB identity changed before report export".to_owned());
    }

    let report = build_report(receipt, expected_source_sha256);
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("serialize Rescue report: {error}"))?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(output_path, [bytes.as_slice(), b"\n"].concat())
        .map_err(|error| format!("write {}: {error}", output_path.display()))?;

    let after = sha256_file(source_path)?;
    if after != before {
        return Err("source PUB identity changed during report export".to_owned());
    }
    Ok(())
}

impl RescueApp {
    fn inspect_source(&mut self) {
        let path = PathBuf::from(self.source_path.trim());
        self.receipt = None;
        self.receipt_status = None;
        self.report_status = None;

        if path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            != Some("pub".to_owned())
        {
            self.source_sha256 = None;
            self.source_status =
                Some("Choose a .pub source file. Chaptera Rescue never overwrites it.".to_owned());
            return;
        }

        match sha256_file(&path) {
            Ok(hash) => {
                self.source_sha256 = Some(hash.clone());
                self.source_status = Some(format!("Source opened read-only. SHA-256: {hash}"));
                if self.report_path.trim().is_empty() {
                    self.report_path = format!("{}.chaptera-rescue-report.json", path.display());
                }
            }
            Err(error) => {
                self.source_sha256 = None;
                self.source_status = Some(error);
            }
        }
    }

    fn load_receipt(&mut self) {
        let Some(source_sha) = self.source_sha256.clone() else {
            self.receipt_status =
                Some("Inspect the source PUB before loading a recovery receipt.".to_owned());
            return;
        };
        let path = PathBuf::from(self.receipt_path.trim());
        match load_product_receipt(&path, &source_sha) {
            Ok(receipt) => {
                let outcome = receipt.outcome.clone();
                self.receipt = Some(receipt);
                self.receipt_status = Some(format!("Validated Chaptera Rescue outcome: {outcome}"));
            }
            Err(error) => {
                self.receipt = None;
                self.receipt_status = Some(error);
            }
        }
    }

    fn export_report(&mut self) {
        let (Some(source_sha), Some(receipt)) = (self.source_sha256.clone(), self.receipt.clone())
        else {
            self.report_status =
                Some("A matching validated Rescue receipt is required before export.".to_owned());
            return;
        };

        let source = PathBuf::from(self.source_path.trim());
        let output = PathBuf::from(self.report_path.trim());
        match write_report(&source, &output, &receipt, &source_sha) {
            Ok(()) => {
                self.report_status = Some(format!(
                    "Wrote source-preserving report: {}",
                    output.display()
                ))
            }
            Err(error) => self.report_status = Some(error),
        }
    }
}

impl eframe::App for RescueApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let Some(path) = file.path else {
                continue;
            };
            match path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
            {
                Some(ext) if ext == "pub" => {
                    self.source_path = path.display().to_string();
                    self.inspect_source();
                }
                Some(ext) if ext == "json" => {
                    self.receipt_path = path.display().to_string();
                }
                _ => {}
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(APP_TITLE);
            ui.label("Damaged-file intake and evidence surface. Recovery execution is not embedded in this technical-preview shell.");
            ui.add_space(12.0);

            ui.group(|ui| {
                ui.strong("1. Source PUB");
                ui.label("Drop a .pub file or enter a local path. The source is read and hashed, never written.");
                ui.text_edit_singleline(&mut self.source_path);
                if ui.button("Inspect source").clicked() {
                    self.inspect_source();
                }
                if let Some(status) = &self.source_status {
                    ui.label(status);
                }
            });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.strong("2. Validated Rescue result");
                ui.label("Load a source-free chaptera.rescue-product-validation.v1 receipt produced by the canonical Rar consumer.");
                ui.text_edit_singleline(&mut self.receipt_path);
                if ui.button("Load validated receipt").clicked() {
                    self.load_receipt();
                }
                if let Some(status) = &self.receipt_status {
                    ui.label(status);
                }
                if let Some(receipt) = &self.receipt {
                    ui.separator();
                    ui.label(format!("Outcome: {}", receipt.outcome));
                    ui.label(format!("Recovered artifacts: {}", receipt.artifact_count));
                    ui.label(format!("Known loss: {}", receipt.known_loss));
                    ui.label(format!(
                        "Native PUB delivery allowed: {}",
                        receipt.native_pub_delivery_allowed
                    ));
                }
            });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.strong("3. Evidence report");
                ui.label("Export writes a separate report and re-hashes the source before and after.");
                ui.text_edit_singleline(&mut self.report_path);
                if ui.button("Export Rescue report").clicked() {
                    self.export_report();
                }
                if let Some(status) = &self.report_status {
                    ui.label(status);
                }
            });
        });
    }
}

fn self_check() -> SelfCheck {
    SelfCheck {
        schema_version: "chaptera.rescue-self-check.v1",
        product_id: PRODUCT_ID,
        canonical_executable: "chaptera-rescue.exe",
        editor_ui_present: false,
        migration_batch_ui_present: false,
        recovery_execution_embedded: false,
    }
}

fn run_cli(args: &[String]) -> Result<Option<i32>, String> {
    if args.iter().any(|arg| arg == "--self-check") {
        println!(
            "{}",
            serde_json::to_string_pretty(&self_check())
                .map_err(|error| format!("serialize self-check: {error}"))?
        );
        return Ok(Some(0));
    }

    if let Some(index) = args.iter().position(|arg| arg == "--acceptance-v1") {
        let values = args.get(index + 1..index + 4).ok_or_else(|| {
            "usage: chaptera-rescue --acceptance-v1 SOURCE.pub PRODUCT-RECEIPT.json REPORT.json"
                .to_owned()
        })?;
        if values.len() != 3 {
            return Err(
                "usage: chaptera-rescue --acceptance-v1 SOURCE.pub PRODUCT-RECEIPT.json REPORT.json"
                    .to_owned(),
            );
        }
        let source = PathBuf::from(&values[0]);
        let receipt_path = PathBuf::from(&values[1]);
        let output = PathBuf::from(&values[2]);
        let source_sha = sha256_file(&source)?;
        let receipt = load_product_receipt(&receipt_path, &source_sha)?;
        write_report(&source, &output, &receipt, &source_sha)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&build_report(&receipt, &source_sha))
                .map_err(|error| format!("serialize acceptance report: {error}"))?
        );
        return Ok(Some(0));
    }

    Ok(None)
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = env::args().collect();
    match run_cli(&args) {
        Ok(Some(code)) => std::process::exit(code),
        Ok(None) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([820.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|_creation_context| Ok(Box::<RescueApp>::default())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(source_sha: &str) -> ProductValidationReceipt {
        ProductValidationReceipt {
            receipt_version: PRODUCT_RECEIPT_VERSION.to_owned(),
            product: "Chaptera Rescue".to_owned(),
            producer_receipt_sha256: "a".repeat(64),
            fixture: FixtureIdentity {
                kind: "natural_partial_cfb".to_owned(),
                source_sha256: source_sha.to_owned(),
            },
            outcome: "partially_recovered".to_owned(),
            source_immutable: true,
            artifact_count: 1,
            known_loss: true,
            native_pub_delivery_allowed: false,
            fail_closed: true,
            privacy: PrivacyState {
                source_free: true,
                private_content_serialized: false,
            },
        }
    }

    #[test]
    fn product_receipt_requires_exact_source_identity() {
        let good = "b".repeat(64);
        validate_product_receipt(&receipt(&good), &good).expect("matching source");
        let error = validate_product_receipt(&receipt(&good), &"c".repeat(64))
            .expect_err("mismatch must fail closed");
        assert!(error.contains("does not match"));
    }

    #[test]
    fn private_product_receipt_is_rejected() {
        let source = "b".repeat(64);
        let mut value = receipt(&source);
        value.privacy.private_content_serialized = true;
        assert!(validate_product_receipt(&value, &source).is_err());
    }

    #[test]
    fn generic_editor_or_migration_identity_is_absent() {
        let cargo = include_str!("../Cargo.toml");
        assert!(!cargo.contains("pub-editor"));
        assert!(!cargo.contains("pub-interaction"));
        assert!(!cargo.contains("chaptera-desktop"));
        let check = self_check();
        assert!(!check.editor_ui_present);
        assert!(!check.migration_batch_ui_present);
        assert!(!check.recovery_execution_embedded);
        assert_eq!(check.canonical_executable, "chaptera-rescue.exe");
    }

    #[test]
    fn report_is_explicit_that_gui_did_not_execute_recovery() {
        let source = "b".repeat(64);
        let report = build_report(&receipt(&source), &source);
        assert!(report.note.contains("does not claim"));
        assert_eq!(report.product_id, "chaptera.rescue");
    }
}
