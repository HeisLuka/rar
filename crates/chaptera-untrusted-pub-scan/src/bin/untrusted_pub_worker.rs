use chaptera_untrusted_pub_scan::{
    PubScanPolicyV1, filesystem_default_deny_supported, inspect_pub_bytes_v1,
    install_post_read_filesystem_default_deny, sha256_hex,
};
use serde::Serialize;
use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Serialize)]
struct FilesystemProbeReceipt {
    protocol_version: &'static str,
    security_profile: &'static str,
    authorized_source_sha256: String,
    authorized_source_bytes: u64,
    filesystem_default_deny: bool,
    read_errno: i32,
    write_errno: i32,
    spawn_errno: i32,
    probes: [&'static str; 3],
}

fn required_env(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("{name} is required"))
}

fn output_file() -> Result<BufWriter<File>, String> {
    let root = PathBuf::from(required_env("CHAPTERA_WORKER_OUTPUT_DIR")?);
    let path = root.join("result.json");
    File::create(&path)
        .map(BufWriter::new)
        .map_err(|error| format!("create {}: {error}", path.display()))
}

fn input_bytes() -> Result<(PathBuf, Vec<u8>), String> {
    let path = PathBuf::from(required_env("CHAPTERA_WORKER_INPUT")?);
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok((path, bytes))
}

fn write_json<T: Serialize>(mut output: BufWriter<File>, value: &T) -> Result<(), String> {
    serde_json::to_writer_pretty(&mut output, value)
        .map_err(|error| format!("serialize result: {error}"))?;
    output
        .write_all(b"\n")
        .map_err(|error| format!("write result newline: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("flush result: {error}"))
}

fn parse_u64(args: &[String], name: &str) -> Result<u64, String> {
    let index = args
        .iter()
        .position(|arg| arg == name)
        .ok_or_else(|| format!("{name} is required"))?;
    let raw = args
        .get(index + 1)
        .ok_or_else(|| format!("{name} value is required"))?;
    raw.parse::<u64>()
        .map_err(|error| format!("invalid {name}: {error}"))
}

fn inspect(args: &[String]) -> Result<(), String> {
    let policy = PubScanPolicyV1 {
        max_file_bytes: parse_u64(args, "--max-file-bytes")?,
        max_cfb_entries: parse_u64(args, "--max-cfb-entries")?,
        max_declared_stream_bytes: parse_u64(args, "--max-declared-stream-bytes")?,
    }
    .validate()?;

    // Open the only publishable result before sealing path-based filesystem access.
    let output = output_file()?;
    let (_source_path, bytes) = input_bytes()?;

    if !filesystem_default_deny_supported() {
        return Err("post-read filesystem confinement is required for this worker".to_owned());
    }
    install_post_read_filesystem_default_deny()
        .map_err(|error| format!("install post-read filesystem sandbox: {error}"))?;

    let result = inspect_pub_bytes_v1(&bytes, policy, true);
    write_json(output, &result)
}

fn expect_eperm(result: std::io::Result<impl Sized>, label: &str) -> Result<i32, String> {
    match result {
        Ok(_) => Err(format!(
            "{label} unexpectedly succeeded after post-read sandbox"
        )),
        Err(error) if error.raw_os_error() == Some(libc::EPERM) => Ok(libc::EPERM),
        Err(error) => Err(format!(
            "{label} was not denied with EPERM: {error} (errno={:?})",
            error.raw_os_error()
        )),
    }
}

fn probe() -> Result<(), String> {
    let output = output_file()?;
    let (_source_path, bytes) = input_bytes()?;
    let source_sha = sha256_hex(&bytes);

    if !filesystem_default_deny_supported() {
        return Err("post-read filesystem confinement is required for this probe".to_owned());
    }
    install_post_read_filesystem_default_deny()
        .map_err(|error| format!("install post-read filesystem sandbox: {error}"))?;

    let read_errno = expect_eperm(fs::read("/etc/hosts"), "read /etc/hosts")?;
    let write_errno = expect_eperm(
        fs::write("/tmp/chaptera-untrusted-pub-escape-probe", b"escape"),
        "write /tmp probe",
    )?;
    let spawn_errno = expect_eperm(Command::new("/bin/true").status(), "spawn /bin/true")?;

    write_json(
        output,
        &FilesystemProbeReceipt {
            protocol_version: "chaptera.untrusted-pub-filesystem-probe.v1",
            security_profile: "chaptera-untrusted-pub-v1",
            authorized_source_sha256: source_sha,
            authorized_source_bytes: bytes.len() as u64,
            filesystem_default_deny: true,
            read_errno,
            write_errno,
            spawn_errno,
            probes: ["read_etc_hosts", "write_tmp", "spawn_true"],
        },
    )
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("inspect") => inspect(&args[1..]),
        Some("probe") => probe(),
        _ => Err("usage: chaptera-untrusted-pub-worker <inspect|probe> ...".to_owned()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
