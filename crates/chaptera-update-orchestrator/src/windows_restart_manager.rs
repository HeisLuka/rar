use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmShutdown, RmStartSession,
};

const MAX_LIST_RETRIES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedProcess {
    pub pid: u32,
    pub terminal_session_id: u32,
    pub application_name: String,
    pub application_status: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionReport {
    pub resource: std::path::PathBuf,
    pub affected: Vec<AffectedProcess>,
    pub reboot_reasons: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuiesceReport {
    pub resource: std::path::PathBuf,
    pub before: Vec<AffectedProcess>,
    pub after: Vec<AffectedProcess>,
    pub reboot_reasons_before: u32,
    pub reboot_reasons_after: u32,
}

struct RestartManagerSession {
    handle: u32,
}

impl RestartManagerSession {
    fn start() -> Result<Self, String> {
        let mut handle = 0_u32;
        let mut session_key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];
        let code = unsafe { RmStartSession(&mut handle, 0, session_key.as_mut_ptr()) };
        check("RmStartSession", code)?;
        Ok(Self { handle })
    }

    fn register_file(&self, path: &Path) -> Result<(), String> {
        let wide = wide_null(path.as_os_str())?;
        let files = [wide.as_ptr()];
        let code = unsafe {
            RmRegisterResources(
                self.handle,
                1,
                files.as_ptr(),
                0,
                null(),
                0,
                null(),
            )
        };
        check("RmRegisterResources", code)
    }

    fn affected_processes(&self) -> Result<(Vec<AffectedProcess>, u32), String> {
        let mut capacity = 0_u32;

        for _ in 0..MAX_LIST_RETRIES {
            let mut needed = 0_u32;
            let mut count = capacity;
            let mut reboot_reasons = 0_u32;
            let mut native = vec![RM_PROCESS_INFO::default(); capacity as usize];
            let native_ptr = if native.is_empty() {
                null_mut()
            } else {
                native.as_mut_ptr()
            };

            let code = unsafe {
                RmGetList(
                    self.handle,
                    &mut needed,
                    &mut count,
                    native_ptr,
                    &mut reboot_reasons,
                )
            };

            if code == ERROR_SUCCESS {
                native.truncate(count as usize);
                let processes = native
                    .into_iter()
                    .map(|item| AffectedProcess {
                        pid: item.Process.dwProcessId,
                        terminal_session_id: item.TSSessionId,
                        application_name: utf16_z(&item.strAppName),
                        application_status: item.AppStatus,
                    })
                    .collect();
                return Ok((processes, reboot_reasons));
            }

            if code == ERROR_MORE_DATA {
                capacity = needed.max(capacity.saturating_mul(2)).max(1);
                continue;
            }

            return Err(format!("RmGetList failed with Win32 error {code}"));
        }

        Err("RmGetList remained unstable while affected processes changed".to_owned())
    }

    fn shutdown_gracefully(&self) -> Result<(), String> {
        let code = unsafe { RmShutdown(self.handle, 0, None) };
        check("RmShutdown", code)
    }
}

impl Drop for RestartManagerSession {
    fn drop(&mut self) {
        unsafe {
            let _ = RmEndSession(self.handle);
        }
    }
}

pub fn inspect_file_resource(resource: &Path) -> Result<InspectionReport, String> {
    let resource = validated_resource_path(resource)?;
    let session = RestartManagerSession::start()?;
    session.register_file(&resource)?;
    let (affected, reboot_reasons) = session.affected_processes()?;
    Ok(InspectionReport {
        resource,
        affected,
        reboot_reasons,
    })
}

pub fn quiesce_file_resource(resource: &Path) -> Result<QuiesceReport, String> {
    let resource = validated_resource_path(resource)?;
    let session = RestartManagerSession::start()?;
    session.register_file(&resource)?;

    let (before, reboot_reasons_before) = session.affected_processes()?;
    if reboot_reasons_before != 0 {
        return Err(format!(
            "Restart Manager requires reboot or user action before shutdown (reasons=0x{reboot_reasons_before:08x})"
        ));
    }

    if !before.is_empty() {
        session.shutdown_gracefully()?;
    }

    let (after, reboot_reasons_after) = session.affected_processes()?;
    if reboot_reasons_after != 0 {
        return Err(format!(
            "Restart Manager requires reboot or user action after shutdown (reasons=0x{reboot_reasons_after:08x})"
        ));
    }
    if !after.is_empty() {
        let pids = after
            .iter()
            .map(|process| process.pid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        return Err(format!(
            "Restart Manager could not prove quiescence; affected processes remain: {pids}"
        ));
    }

    Ok(QuiesceReport {
        resource,
        before,
        after,
        reboot_reasons_before,
        reboot_reasons_after,
    })
}

fn validated_resource_path(resource: &Path) -> Result<std::path::PathBuf, String> {
    let resource = std::path::absolute(resource).map_err(|error| {
        format!(
            "resolve absolute Restart Manager resource {}: {error}",
            resource.display()
        )
    })?;
    if !resource.is_file() {
        return Err(format!(
            "Restart Manager resource is not a file: {}",
            resource.display()
        ));
    }
    Ok(resource)
}

fn wide_null(value: &OsStr) -> Result<Vec<u16>, String> {
    let mut wide = value.encode_wide().collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err("Restart Manager resource path contains an interior NUL".to_owned());
    }
    wide.push(0);
    Ok(wide)
}

fn utf16_z(value: &[u16]) -> String {
    let length = value.iter().position(|code_unit| *code_unit == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

fn check(operation: &str, code: u32) -> Result<(), String> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation} failed with Win32 error {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_z_stops_at_first_nul() {
        assert_eq!(utf16_z(&[b'A' as u16, b'B' as u16, 0, b'C' as u16]), "AB");
    }

    #[test]
    fn wide_null_appends_exact_terminator() {
        let wide = wide_null(OsStr::new(r"C:\\Chaptera\\current\\chaptera-reader.exe")).unwrap();
        assert_eq!(wide.last(), Some(&0));
        assert_eq!(wide.iter().filter(|value| **value == 0).count(), 1);
    }


    #[test]
    fn current_executable_preflight_detects_the_current_process_without_shutdown() {
        let executable = std::env::current_exe().expect("current test executable");
        let report = inspect_file_resource(&executable)
            .expect("Restart Manager self preflight should be inspectable");
        let current_pid = std::process::id();
        assert!(
            report.affected.iter().any(|process| process.pid == current_pid)
                || report.reboot_reasons != 0,
            "Restart Manager preflight neither listed current PID {current_pid} nor reported a reboot/self boundary: {report:?}"
        );
    }
}
