use chaptera_update_candidate::{
    extract_verified_reader_candidate, verify_candidate_tree,
};
use chaptera_update_trust::VerifiedPayloadReceipt;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut out);
        for (name, bytes) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn receipt(payload: &[u8], installed_tree_bytes: u64) -> VerifiedPayloadReceipt {
    VerifiedPayloadReceipt {
        schema_version: "chaptera.verified-payload.v1".into(),
        target_name: "Chaptera-Reader-Windows-x86_64.zip".into(),
        product_id: "chaptera.reader".into(),
        architecture: "windows-x86_64".into(),
        channel: "stable".into(),
        package_version: "0.2.0".into(),
        payload_sha256: Sha256::digest(payload)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        payload_byte_len: payload.len() as u64,
        installed_tree_bytes,
        timestamp_version: 2,
        snapshot_version: 2,
        targets_version: 2,
    }
}

#[test]
fn extracts_only_the_canonical_reader_tree_and_reverifies_it() {
    let exe = b"MZ-reader-v2";
    let readme = b"chaptera.reader-portable-readme.v1";
    let payload = make_zip(&[
        ("Chaptera-Reader.exe", exe),
        ("README.md", readme),
    ]);
    let temp = tempdir().unwrap();
    let root = temp.path().join("candidate");

    let candidate = extract_verified_reader_candidate(
        &payload,
        &receipt(&payload, (exe.len() + readme.len()) as u64),
        &root,
    )
    .unwrap();

    assert_eq!(candidate.files.len(), 2);
    verify_candidate_tree(&root, &candidate).unwrap();

    std::fs::write(root.join("README.md"), b"tampered").unwrap();
    assert!(verify_candidate_tree(&root, &candidate).is_err());
}

#[test]
fn rejects_extra_entry_and_tree_size_mismatch() {
    let exe = b"MZ-reader-v2";
    let readme = b"chaptera.reader-portable-readme.v1";
    let extra = make_zip(&[
        ("Chaptera-Reader.exe", exe),
        ("README.md", readme),
        ("evil.dll", b"evil"),
    ]);
    let temp = tempdir().unwrap();
    assert!(extract_verified_reader_candidate(
        &extra,
        &receipt(&extra, (exe.len() + readme.len() + 4) as u64),
        &temp.path().join("extra"),
    )
    .is_err());

    let valid = make_zip(&[
        ("Chaptera-Reader.exe", exe),
        ("README.md", readme),
    ]);
    assert!(extract_verified_reader_candidate(
        &valid,
        &receipt(&valid, 1),
        &temp.path().join("wrong-size"),
    )
    .is_err());
}

#[test]
fn rejects_duplicate_and_nested_entries() {
    let exe = b"MZ-reader-v2";
    let readme = b"chaptera.reader-portable-readme.v1";
    let duplicate = make_zip(&[
        ("Chaptera-Reader.exe", exe),
        ("Chaptera-Reader.exe", exe),
    ]);
    let temp = tempdir().unwrap();
    assert!(extract_verified_reader_candidate(
        &duplicate,
        &receipt(&duplicate, (exe.len() * 2) as u64),
        &temp.path().join("duplicate"),
    )
    .is_err());

    let nested = make_zip(&[
        ("Chaptera-Reader.exe", exe),
        ("../README.md", readme),
    ]);
    assert!(extract_verified_reader_candidate(
        &nested,
        &receipt(&nested, (exe.len() + readme.len()) as u64),
        &temp.path().join("nested"),
    )
    .is_err());
}
