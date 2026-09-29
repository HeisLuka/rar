use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{diagnostic_sweep, reader_only_mode};

pub(super) fn try_handle(
    first_arg: Option<&OsStr>,
    args: &mut impl Iterator<Item = OsString>,
) -> bool {
    if first_arg == Some(OsStr::new(chaptera_update_handoff::CONTROL_MODE_ARG)) {
        if !reader_only_mode() {
            eprintln!("update control mode is reserved for the Chaptera Reader product");
            std::process::exit(2);
        }
        let Some(request_path) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera-reader --chaptera-update-control HANDOFF-REQUEST.json");
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("Reader update control mode accepts exactly one handoff request");
            std::process::exit(2);
        }
        if let Err(error) = run_reader_update_control(&request_path) {
            eprintln!("Reader update control failed: {error}");
            std::process::exit(2);
        }
        return true;
    }

    if first_arg == Some(OsStr::new("--reader-activation-probe-v1")) {
        if !reader_only_mode() {
            eprintln!("Reader activation probe is reserved for the Reader build");
            std::process::exit(2);
        }
        let Some(path) = args.next().map(PathBuf::from) else {
            eprintln!(
                "usage: chaptera-reader --reader-activation-probe-v1 SOURCE.pub RECEIPT.json HOLD_MS"
            );
            std::process::exit(2);
        };
        let Some(receipt) = args.next().map(PathBuf::from) else {
            eprintln!(
                "usage: chaptera-reader --reader-activation-probe-v1 SOURCE.pub RECEIPT.json HOLD_MS"
            );
            std::process::exit(2);
        };
        let Some(hold_ms) = args
            .next()
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.parse::<u64>().ok())
        else {
            eprintln!("Reader activation probe HOLD_MS must be an integer");
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("Reader activation probe accepts exactly source, receipt, and hold_ms");
            std::process::exit(2);
        }
        if let Err(error) = reader_activation_probe(&path, &receipt, hold_ms) {
            eprintln!("Reader activation probe failed: {error}");
            std::process::exit(2);
        }
        return true;
    }

    false
}

#[cfg(feature = "reader-only")]
struct ReaderControlHooks {
    install_root: PathBuf,
    reader_relative_path: PathBuf,
}

#[cfg(feature = "reader-only")]
impl chaptera_update_orchestrator::UpdateHooks for ReaderControlHooks {
    fn quiesce(&mut self, control_updater: &Path) -> std::result::Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            let reader = self
                .install_root
                .join("current")
                .join(&self.reader_relative_path);
            if control_updater == reader {
                return Err(
                    "copied control updater must execute outside the active Reader tree".to_owned(),
                );
            }
            let report =
                chaptera_update_orchestrator::windows_restart_manager::quiesce_file_resource(
                    &reader,
                )?;
            let affected_pids = report
                .before
                .iter()
                .map(|process| process.pid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            eprintln!(
                "CHAPTERA_READER_RM_QUIESCE resource={} affected_before={} affected_pids={} affected_after={} reboot_before=0x{:08x} reboot_after=0x{:08x}",
                report.resource.display(),
                report.before.len(),
                affected_pids,
                report.after.len(),
                report.reboot_reasons_before,
                report.reboot_reasons_after,
            );
            Ok(())
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = control_updater;
            Err("Reader update quiesce requires Windows Restart Manager".to_owned())
        }
    }

    fn health_check(&mut self, current_tree: &Path) -> std::result::Result<(), String> {
        use sha2::{Digest, Sha256};
        use std::process::{Command, Stdio};
        use std::thread;

        const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);

        let candidate = current_tree.join(
            std::env::current_exe()
                .map_err(|error| format!("resolve control executable: {error}"))?
                .file_name()
                .ok_or_else(|| "control executable has no file name".to_owned())?,
        );
        let bytes = fs::read(&candidate)
            .map_err(|error| format!("read activated Reader {}: {error}", candidate.display()))?;
        if bytes.is_empty() {
            return Err(format!(
                "activated Reader executable is empty: {}",
                candidate.display()
            ));
        }
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let mut child = Command::new(&candidate)
            .arg("--product-smoke-v1")
            .env("CHAPTERA_PRODUCT_SMOKE_BINARY_SHA256", sha256)
            .env(
                "CHAPTERA_PRODUCT_SMOKE_BINARY_BYTE_LEN",
                bytes.len().to_string(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("launch activated Reader health smoke: {error}"))?;

        let deadline = Instant::now() + HEALTH_TIMEOUT;
        loop {
            match child
                .try_wait()
                .map_err(|error| format!("wait for activated Reader health smoke: {error}"))?
            {
                Some(status) if status.success() => return Ok(()),
                Some(status) => {
                    return Err(format!(
                        "activated Reader health smoke failed with status {status}"
                    ));
                }
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "activated Reader health smoke exceeded {} seconds",
                        HEALTH_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }
}

#[cfg(feature = "reader-only")]
fn run_reader_update_control(request_path: &Path) -> Result<(), String> {
    let request = chaptera_update_handoff::read_control_request(request_path)
        .map_err(|error| error.to_string())?;
    let orchestrator = chaptera_update_orchestrator::UpdateOrchestrator::new(&request.install_root);
    chaptera_update_handoff::validate_request_against_engine(&request, orchestrator.engine())
        .map_err(|error| error.to_string())?;

    let _lock = chaptera_update_orchestrator::InstallLock::acquire(&request.install_root)
        .map_err(|error| error.to_string())?;
    chaptera_update_handoff::validate_request_against_engine(&request, orchestrator.engine())
        .map_err(|error| error.to_string())?;

    let receipt = chaptera_update_handoff::ControlReceipt {
        schema_version: chaptera_update_handoff::CONTROL_RECEIPT_SCHEMA_VERSION.to_owned(),
        transaction_id: request.transaction_id.clone(),
        pid: std::process::id(),
        executable: std::env::current_exe()
            .map_err(|error| format!("resolve control executable: {error}"))?,
    };
    chaptera_update_handoff::write_control_receipt(
        &chaptera_update_handoff::receipt_path(request_path),
        &receipt,
    )
    .map_err(|error| error.to_string())?;

    let mut hooks = ReaderControlHooks {
        install_root: request.install_root.clone(),
        reader_relative_path: request.updater_relative_path.clone(),
    };
    orchestrator
        .continue_prepared_candidate(&mut hooks)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(not(feature = "reader-only"))]
fn run_reader_update_control(_request_path: &Path) -> Result<(), String> {
    Err("update control mode is unavailable outside the Reader build".to_owned())
}

#[cfg(target_os = "windows")]
static READER_RM_PROBE_SHUTDOWN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "windows")]
unsafe extern "system" fn reader_rm_probe_window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION,
    };

    match message {
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION if wparam != 0 => {
            READER_RM_PROBE_SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);
            0
        }
        WM_CLOSE => {
            READER_RM_PROBE_SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(target_os = "windows")]
fn hold_reader_activation_for_restart_manager<F>(hold_ms: u64, ready: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
        RegisterClassW, TranslateMessage, UnregisterClassW, WNDCLASSW,
    };

    READER_RM_PROBE_SHUTDOWN.store(false, std::sync::atomic::Ordering::SeqCst);
    let class_name = "ChapteraReaderRmProbeWindowV1\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    let window_title = "Chaptera Reader Restart Manager probe\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        return Err("GetModuleHandleW failed for Reader RM probe".to_owned());
    }

    let mut window_class: WNDCLASSW = unsafe { std::mem::zeroed() };
    window_class.lpfnWndProc = Some(reader_rm_probe_window_proc);
    window_class.hInstance = instance;
    window_class.lpszClassName = class_name.as_ptr();
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err("RegisterClassW failed for Reader RM probe".to_owned());
    }

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            0,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        unsafe {
            UnregisterClassW(class_name.as_ptr(), instance);
        }
        return Err("CreateWindowExW failed for Reader RM probe".to_owned());
    }

    if let Err(error) = ready() {
        unsafe {
            DestroyWindow(hwnd);
            UnregisterClassW(class_name.as_ptr(), instance);
        }
        return Err(error);
    }

    let deadline = Instant::now() + Duration::from_millis(hold_ms);
    while !READER_RM_PROBE_SHUTDOWN.load(std::sync::atomic::Ordering::SeqCst)
        && Instant::now() < deadline
    {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    unsafe {
        DestroyWindow(hwnd);
        UnregisterClassW(class_name.as_ptr(), instance);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn hold_reader_activation_for_restart_manager<F>(hold_ms: u64, ready: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    ready()?;
    std::thread::sleep(Duration::from_millis(hold_ms));
    Ok(())
}

fn reader_activation_probe(path: &Path, receipt: &Path, hold_ms: u64) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    const MAX_HOLD_MS: u64 = 60_000;
    if hold_ms == 0 || hold_ms > MAX_HOLD_MS {
        return Err(format!(
            "hold_ms must be within 1..={MAX_HOLD_MS}, got {hold_ms}"
        ));
    }

    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let source_sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let visual = diagnostic_sweep::open_for_product(&bytes)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let page_count = visual.document.pages.len();

    let write_receipt = |completed: bool, source_unchanged: bool| -> Result<(), String> {
        if let Some(parent) = receipt.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create activation receipt parent: {error}"))?;
        }
        let value = serde_json::json!({
            "schema_version": "chaptera.reader-activation-session.v1",
            "pid": std::process::id(),
            "source_sha256": source_sha256,
            "source_byte_len": bytes.len(),
            "page_count": page_count,
            "read_only": true,
            "process_model": "independent_process_per_activation",
            "completed": completed,
            "source_unchanged": source_unchanged,
        });
        fs::write(
            receipt,
            format!(
                "{}\n",
                serde_json::to_string(&value)
                    .map_err(|error| format!("serialize activation receipt: {error}"))?
            ),
        )
        .map_err(|error| format!("write activation receipt {}: {error}", receipt.display()))
    };

    hold_reader_activation_for_restart_manager(hold_ms, || write_receipt(false, false))?;
    std::hint::black_box(&visual);

    let after = fs::read(path)
        .map_err(|error| format!("re-read {} after activation hold: {error}", path.display()))?;
    let after_sha256 = Sha256::digest(&after)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if after_sha256 != source_sha256 || after.len() != bytes.len() {
        return Err("Reader activation probe observed source mutation".to_owned());
    }
    write_receipt(true, true)
}
