use chaptera_update_engine::staging::{
    stage_verified_zip, validate_manifest, ManifestFile, StagingLimits, TreeManifest,
};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn limits() -> StagingLimits {
    StagingLimits {
        max_files: 16,
        max_installed_bytes: 1024 * 1024,
        temp_overhead_bytes: 0,
        retained_predecessor_bytes: 0,
    }
}

fn manifest(files: &[(&str, &[u8])]) -> TreeManifest {
    TreeManifest {
        files: files
            .iter()
            .map(|(path, bytes)| ManifestFile {
                path: (*path).to_owned(),
                length: bytes.len() as u64,
                sha256: digest(bytes),
            })
            .collect(),
        file_count: files.len() as u64,
        installed_tree_bytes: files.iter().map(|(_, bytes)| bytes.len() as u64).sum(),
    }
}

fn write_zip(path: &std::path::Path, files: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (name, bytes) in files {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn stages_exact_authenticated_tree_and_allows_zero_length_file() {
    let temp = tempdir().unwrap();
    let install = temp.path().join("install");
    let archive = temp.path().join("candidate.zip");
    let destination = install.join(".staging/tx/candidate");
    let files: &[(&str, &[u8])] = &[
        ("Reader.exe", b"reader"),
        ("assets/empty.dat", b""),
    ];
    write_zip(&archive, files);
    let manifest = manifest(files);

    stage_verified_zip(&install, &archive, &manifest, limits(), &destination).unwrap();

    assert_eq!(fs::read(destination.join("Reader.exe")).unwrap(), b"reader");
    assert_eq!(fs::read(destination.join("assets/empty.dat")).unwrap(), b"");
}

#[test]
fn manifest_rejects_traversal_reserved_names_and_case_collisions() {
    for bad in ["../escape.exe", "C:/escape.exe", "dir\\escape.exe", "CON", "aux.txt", "name."] {
        let m = TreeManifest {
            files: vec![ManifestFile {
                path: bad.into(),
                length: 1,
                sha256: digest(b"x"),
            }],
            file_count: 1,
            installed_tree_bytes: 1,
        };
        assert!(validate_manifest(&m, limits()).is_err(), "{bad} must be rejected");
    }

    let collision = TreeManifest {
        files: vec![
            ManifestFile { path: "Reader.exe".into(), length: 1, sha256: digest(b"a") },
            ManifestFile { path: "reader.EXE".into(), length: 1, sha256: digest(b"b") },
        ],
        file_count: 2,
        installed_tree_bytes: 2,
    };
    assert!(validate_manifest(&collision, limits()).is_err());
}

#[test]
fn rejects_extra_missing_and_tampered_payloads_without_publishing_destination() {
    let scenarios: Vec<(Vec<(&str, &[u8])>, TreeManifest)> = vec![
        (
            vec![("Reader.exe", b"reader"), ("extra.dll", b"x")],
            manifest(&[("Reader.exe", b"reader")]),
        ),
        (
            vec![("Reader.exe", b"reader")],
            manifest(&[("Reader.exe", b"reader"), ("needed.dll", b"n")]),
        ),
        (
            vec![("Reader.exe", b"tampered")],
            manifest(&[("Reader.exe", b"reader")]),
        ),
    ];

    for (index, (zip_files, expected)) in scenarios.into_iter().enumerate() {
        let temp = tempdir().unwrap();
        let install = temp.path().join("install");
        let archive = temp.path().join(format!("candidate-{index}.zip"));
        let destination = install.join(".staging/tx/candidate");
        write_zip(&archive, &zip_files);
        assert!(stage_verified_zip(&install, &archive, &expected, limits(), &destination).is_err());
        assert!(!destination.exists());
    }
}

#[test]
fn manifest_bounds_fail_before_extraction() {
    let m = manifest(&[("Reader.exe", b"reader")]);
    let too_few = StagingLimits { max_files: 0, ..limits() };
    assert!(validate_manifest(&m, too_few).is_err());

    let too_small = StagingLimits { max_installed_bytes: 1, ..limits() };
    assert!(validate_manifest(&m, too_small).is_err());
}
