use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;

pub const SECURITY_PROFILE_V1: &str = "chaptera-untrusted-pub-v1";
pub const DEFAULT_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_MAX_CFB_ENTRIES: u64 = 8_192;
pub const DEFAULT_MAX_DECLARED_STREAM_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubScanPolicyV1 {
    pub max_file_bytes: u64,
    pub max_cfb_entries: u64,
    pub max_declared_stream_bytes: u64,
}

impl Default for PubScanPolicyV1 {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_cfb_entries: DEFAULT_MAX_CFB_ENTRIES,
            max_declared_stream_bytes: DEFAULT_MAX_DECLARED_STREAM_BYTES,
        }
    }
}

impl PubScanPolicyV1 {
    pub fn validate(self) -> Result<Self, String> {
        if self.max_file_bytes == 0 {
            return Err("max_file_bytes must be positive".to_owned());
        }
        if self.max_cfb_entries == 0 {
            return Err("max_cfb_entries must be positive".to_owned());
        }
        if self.max_declared_stream_bytes == 0 {
            return Err("max_declared_stream_bytes must be positive".to_owned());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubScanStatusV1 {
    AcceptedCfb,
    ParseFailed,
    RejectedByPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubScanResultV1 {
    pub protocol_version: String,
    pub security_profile: String,
    pub status: PubScanStatusV1,
    pub byte_len: u64,
    pub sha256: String,
    pub cfb_entry_count: Option<u64>,
    pub declared_stream_bytes: Option<u64>,
    pub filesystem_confinement: bool,
    pub security_event: Option<String>,
    pub error: Option<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn inspect_pub_bytes_v1(
    bytes: &[u8],
    policy: PubScanPolicyV1,
    filesystem_confinement: bool,
) -> PubScanResultV1 {
    let policy = match policy.validate() {
        Ok(policy) => policy,
        Err(error) => {
            return result(
                bytes,
                PubScanStatusV1::RejectedByPolicy,
                None,
                None,
                filesystem_confinement,
                Some("invalid_policy".to_owned()),
                Some(error),
            );
        }
    };

    let byte_len = bytes.len() as u64;
    if byte_len > policy.max_file_bytes {
        return result(
            bytes,
            PubScanStatusV1::RejectedByPolicy,
            None,
            None,
            filesystem_confinement,
            Some(format!(
                "input_size_limit: {byte_len} > {}",
                policy.max_file_bytes
            )),
            Some("PUB input exceeds admitted byte limit".to_owned()),
        );
    }

    let compound = match cfb::CompoundFile::open(Cursor::new(bytes)) {
        Ok(compound) => compound,
        Err(error) => {
            return result(
                bytes,
                PubScanStatusV1::ParseFailed,
                None,
                None,
                filesystem_confinement,
                None,
                Some(format!("CFB parse failed: {error}")),
            );
        }
    };

    let mut entry_count = 0_u64;
    let mut declared_stream_bytes = 0_u64;
    for entry in compound.walk() {
        entry_count = match entry_count.checked_add(1) {
            Some(value) => value,
            None => {
                return result(
                    bytes,
                    PubScanStatusV1::RejectedByPolicy,
                    None,
                    None,
                    filesystem_confinement,
                    Some("cfb_entry_count_overflow".to_owned()),
                    Some("CFB entry count overflowed u64".to_owned()),
                );
            }
        };
        if entry_count > policy.max_cfb_entries {
            return result(
                bytes,
                PubScanStatusV1::RejectedByPolicy,
                Some(entry_count),
                None,
                filesystem_confinement,
                Some(format!(
                    "cfb_entry_limit: {entry_count} > {}",
                    policy.max_cfb_entries
                )),
                Some("CFB entry count exceeds admitted limit".to_owned()),
            );
        }

        if entry.is_stream() {
            declared_stream_bytes = match declared_stream_bytes.checked_add(entry.len()) {
                Some(value) => value,
                None => {
                    return result(
                        bytes,
                        PubScanStatusV1::RejectedByPolicy,
                        Some(entry_count),
                        None,
                        filesystem_confinement,
                        Some("cfb_declared_stream_bytes_overflow".to_owned()),
                        Some("CFB declared stream byte sum overflowed u64".to_owned()),
                    );
                }
            };
            if declared_stream_bytes > policy.max_declared_stream_bytes {
                return result(
                    bytes,
                    PubScanStatusV1::RejectedByPolicy,
                    Some(entry_count),
                    Some(declared_stream_bytes),
                    filesystem_confinement,
                    Some(format!(
                        "cfb_declared_stream_bytes_limit: {declared_stream_bytes} > {}",
                        policy.max_declared_stream_bytes
                    )),
                    Some("CFB declared stream bytes exceed admitted limit".to_owned()),
                );
            }
        }
    }

    result(
        bytes,
        PubScanStatusV1::AcceptedCfb,
        Some(entry_count),
        Some(declared_stream_bytes),
        filesystem_confinement,
        None,
        None,
    )
}

fn result(
    bytes: &[u8],
    status: PubScanStatusV1,
    cfb_entry_count: Option<u64>,
    declared_stream_bytes: Option<u64>,
    filesystem_confinement: bool,
    security_event: Option<String>,
    error: Option<String>,
) -> PubScanResultV1 {
    PubScanResultV1 {
        protocol_version: "chaptera.untrusted-pub-scan-result.v1".to_owned(),
        security_profile: SECURITY_PROFILE_V1.to_owned(),
        status,
        byte_len: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        cfb_entry_count,
        declared_stream_bytes,
        filesystem_confinement,
        security_event,
        error,
    }
}

pub fn filesystem_default_deny_supported() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

pub fn install_post_read_filesystem_default_deny() -> std::io::Result<()> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        linux::install_filesystem_default_deny()
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "post-read filesystem default-deny is implemented only on Linux x86_64",
        ))
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux {
    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JMP_JEQ_K: u16 = 0x15;
    const BPF_RET_K: u16 = 0x06;
    const SECCOMP_DATA_NR_OFFSET: u32 = 0;
    const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
    const SECCOMP_MODE_FILTER: libc::c_ulong = 2;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

    pub(super) fn install_filesystem_default_deny() -> std::io::Result<()> {
        let denied_syscalls: &[libc::c_long] = &[
            libc::SYS_open,
            libc::SYS_openat,
            libc::SYS_openat2,
            libc::SYS_creat,
            libc::SYS_open_by_handle_at,
            libc::SYS_stat,
            libc::SYS_lstat,
            libc::SYS_newfstatat,
            libc::SYS_statx,
            libc::SYS_access,
            libc::SYS_faccessat,
            libc::SYS_faccessat2,
            libc::SYS_readlink,
            libc::SYS_readlinkat,
            libc::SYS_getdents,
            libc::SYS_getdents64,
            libc::SYS_unlink,
            libc::SYS_unlinkat,
            libc::SYS_rename,
            libc::SYS_renameat,
            libc::SYS_renameat2,
            libc::SYS_link,
            libc::SYS_linkat,
            libc::SYS_symlink,
            libc::SYS_symlinkat,
            libc::SYS_mkdir,
            libc::SYS_mkdirat,
            libc::SYS_rmdir,
            libc::SYS_mknod,
            libc::SYS_mknodat,
            libc::SYS_chmod,
            libc::SYS_fchmod,
            libc::SYS_fchmodat,
            libc::SYS_chown,
            libc::SYS_fchown,
            libc::SYS_lchown,
            libc::SYS_fchownat,
            libc::SYS_truncate,
            libc::SYS_ftruncate,
            libc::SYS_utime,
            libc::SYS_utimes,
            libc::SYS_futimesat,
            libc::SYS_utimensat,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_fork,
            libc::SYS_vfork,
            libc::SYS_clone,
            libc::SYS_clone3,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
        ];
        install_errno_filter(denied_syscalls)
    }

    fn install_errno_filter(denied_syscalls: &[libc::c_long]) -> std::io::Result<()> {
        let mut filter = Vec::with_capacity(5 + denied_syscalls.len() * 2);
        filter.push(statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH_OFFSET));
        filter.push(jump(BPF_JMP_JEQ_K, AUDIT_ARCH_X86_64, 1, 0));
        filter.push(statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
        filter.push(statement(BPF_LD_W_ABS, SECCOMP_DATA_NR_OFFSET));

        for &syscall_number in denied_syscalls {
            filter.push(jump(BPF_JMP_JEQ_K, syscall_number as u32, 0, 1));
            filter.push(statement(
                BPF_RET_K,
                SECCOMP_RET_ERRNO | (libc::EPERM as u32),
            ));
        }
        filter.push(statement(BPF_RET_K, SECCOMP_RET_ALLOW));

        let mut program = libc::sock_fprog {
            len: filter
                .len()
                .try_into()
                .expect("seccomp filter length must fit u16"),
            filter: filter.as_mut_ptr(),
        };

        let no_new_privs = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if no_new_privs != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = unsafe {
            libc::prctl(
                libc::PR_SET_SECCOMP,
                SECCOMP_MODE_FILTER,
                &mut program as *mut libc::sock_fprog,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn statement(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_identity_is_stable() {
        assert_eq!(
            sha256_hex(b"pub"),
            "0017dea7770f7ecff7ab3c20506546129e96bdeba2f544bb8e5414eb79786122"
        );
    }

    #[test]
    fn invalid_cfb_is_a_parse_failure_not_a_policy_acceptance() {
        let result = inspect_pub_bytes_v1(b"not a cfb", PubScanPolicyV1::default(), false);
        assert_eq!(result.status, PubScanStatusV1::ParseFailed);
        assert!(result.sha256.len() == 64);
    }

    #[test]
    fn input_size_limit_is_enforced_before_cfb_parse() {
        let result = inspect_pub_bytes_v1(
            b"12345",
            PubScanPolicyV1 {
                max_file_bytes: 4,
                ..PubScanPolicyV1::default()
            },
            false,
        );
        assert_eq!(result.status, PubScanStatusV1::RejectedByPolicy);
        assert!(
            result
                .security_event
                .as_deref()
                .is_some_and(|value| value.starts_with("input_size_limit:"))
        );
    }

    #[test]
    fn platform_claim_matches_implemented_post_read_sandbox() {
        assert_eq!(
            filesystem_default_deny_supported(),
            cfg!(all(target_os = "linux", target_arch = "x86_64"))
        );
    }
}
