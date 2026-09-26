#![cfg(target_os = "windows")]

use crate::{
    DEVICE_KEY_ID_LEN, LeaseTimeInputV1, TimeAcceptance, TimePolicy, TrustedTimeError,
    TrustedTimeStateV1, evaluate_time_bound_right,
};
use crate::activation_request::{
    ACTIVATION_REQUEST_SIGNATURE_DOMAIN, ActivationRequestFactsV1, assemble_activation_request,
    encode_activation_request_facts,
};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, c_void};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, LocalFree, NTE_BAD_KEYSET, NTE_EXISTS, WAIT_ABANDONED,
    WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALGORITHM, BCRYPT_ECDSA_PUBLIC_P256_MAGIC,
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    MS_KEY_STORAGE_PROVIDER, MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_KEY_HANDLE, NCRYPT_PROV_HANDLE,
    NCRYPT_SILENT_FLAG, NCryptCreatePersistedKey, NCryptExportKey,
    NCryptFinalizeKey, NCryptFreeObject, NCryptOpenKey, NCryptOpenStorageProvider, NCryptSetProperty,
    NCryptSignHash,
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows_sys::core::PCWSTR;

const POSSESSION_DOMAIN: &[u8] = b"Chaptera.DeviceKey.possession.v1\0";
const STATE_ENTROPY_DOMAIN: &[u8] = b"Chaptera.TrustedTimeState.dpapi.v1\0";
const STATE_MUTEX_DOMAIN: &[u8] = b"Chaptera.TrustedTimeState.mutex.v1\0";
const STATE_MUTEX_WAIT_MS: u32 = 30_000;
const P256_PUBLIC_BLOB_LEN: usize = 8 + 32 + 32;
const NTE_NOT_SUPPORTED_STATUS: u32 = 0x8009_0029;
const NTE_DEVICE_NOT_READY_STATUS: u32 = 0x8009_0030;
const NTE_DEVICE_NOT_FOUND_STATUS: u32 = 0x8009_0035;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKeyBacking {
    HardwareTpm,
    SoftwareKsp,
}

#[derive(Debug, Error)]
pub enum WindowsPlatformError {
    #[error("{api} failed with CNG status 0x{status:08x}")]
    Cng { api: &'static str, status: u32 },
    #[error("{api} failed with Windows error {code}")]
    Win32 { api: &'static str, code: u32 },
    #[error("CNG returned an invalid P-256 public key blob")]
    InvalidPublicKeyBlob,
    #[error("CNG returned an invalid P-256 signature")]
    InvalidSignature,
    #[error("DeviceKey possession proof failed")]
    PossessionProofFailed,
    #[error("persisted DeviceKey is missing from pinned backing {0:?}")]
    DeviceKeyMissing(DeviceKeyBacking),
    #[error("protected TrustedTime state belongs to another DeviceKey")]
    StateDeviceMismatch,
    #[error("TrustedTime state CBOR encode failed: {0}")]
    StateEncode(String),
    #[error("TrustedTime state CBOR decode failed: {0}")]
    StateDecode(String),
    #[error("path has no file name")]
    InvalidStatePath,
    #[error("TrustedTime state transaction lock timed out")]
    StateLockTimeout,
    #[error("TrustedTime policy rejected the candidate: {0}")]
    TrustedTime(#[from] TrustedTimeError),
    #[error("activation request creation failed: {0}")]
    ActivationRequest(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

struct KeyHandles {
    provider: NCRYPT_PROV_HANDLE,
    key: NCRYPT_KEY_HANDLE,
}

impl Drop for KeyHandles {
    fn drop(&mut self) {
        unsafe {
            if self.key != 0 {
                let _ = NCryptFreeObject(self.key);
            }
            if self.provider != 0 {
                let _ = NCryptFreeObject(self.provider);
            }
        }
    }
}

pub struct WindowsDeviceKey {
    handles: KeyHandles,
    backing: DeviceKeyBacking,
}

impl WindowsDeviceKey {
    /// Bootstrap a DeviceKey backing once. Existing software fallback wins so
    /// a machine does not silently migrate back to TPM later. If no key exists,
    /// TPM is preferred; a software key is created only when the TPM provider
    /// is unavailable/unsupported during bootstrap.
    ///
    /// After bootstrap, persist `backing()` and use `open_existing` on later
    /// launches. That pins identity and prevents a transient TPM failure from
    /// silently creating a different software DeviceKey.
    pub fn open_or_create(key_name: &str) -> Result<Self, WindowsPlatformError> {
        let key_name = wide(key_name);

        if let Some(existing) = try_open_existing(
            MS_KEY_STORAGE_PROVIDER,
            DeviceKeyBacking::SoftwareKsp,
            &key_name,
        )? {
            return Ok(existing);
        }

        match try_open_existing(
            MS_PLATFORM_CRYPTO_PROVIDER,
            DeviceKeyBacking::HardwareTpm,
            &key_name,
        ) {
            Ok(Some(existing)) => return Ok(existing),
            Ok(None) => {
                match create_with_provider(
                    MS_PLATFORM_CRYPTO_PROVIDER,
                    DeviceKeyBacking::HardwareTpm,
                    &key_name,
                ) {
                    Ok(created) => return Ok(created),
                    Err(error) if is_bootstrap_tpm_unavailable(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            Err(error) if is_bootstrap_tpm_unavailable(&error) => {}
            Err(error) => return Err(error),
        }

        create_with_provider(
            MS_KEY_STORAGE_PROVIDER,
            DeviceKeyBacking::SoftwareKsp,
            &key_name,
        )
    }

    pub fn open_existing(
        key_name: &str,
        backing: DeviceKeyBacking,
    ) -> Result<Self, WindowsPlatformError> {
        let key_name = wide(key_name);
        let provider = match backing {
            DeviceKeyBacking::HardwareTpm => MS_PLATFORM_CRYPTO_PROVIDER,
            DeviceKeyBacking::SoftwareKsp => MS_KEY_STORAGE_PROVIDER,
        };
        try_open_existing(provider, backing, &key_name)?
            .ok_or(WindowsPlatformError::DeviceKeyMissing(backing))
    }

    pub fn backing(&self) -> DeviceKeyBacking {
        self.backing
    }

    pub fn public_key_sec1(&self) -> Result<[u8; 65], WindowsPlatformError> {
        let mut required = 0_u32;
        let status = unsafe {
            NCryptExportKey(
                self.handles.key,
                0,
                BCRYPT_ECCPUBLIC_BLOB,
                null(),
                null_mut(),
                0,
                &mut required,
                NCRYPT_SILENT_FLAG,
            )
        };
        cng_ok("NCryptExportKey(size)", status)?;

        if required as usize != P256_PUBLIC_BLOB_LEN {
            return Err(WindowsPlatformError::InvalidPublicKeyBlob);
        }

        let mut blob = vec![0_u8; required as usize];
        let mut written = 0_u32;
        let status = unsafe {
            NCryptExportKey(
                self.handles.key,
                0,
                BCRYPT_ECCPUBLIC_BLOB,
                null(),
                blob.as_mut_ptr(),
                blob.len() as u32,
                &mut written,
                NCRYPT_SILENT_FLAG,
            )
        };
        cng_ok("NCryptExportKey", status)?;
        if written as usize != P256_PUBLIC_BLOB_LEN {
            return Err(WindowsPlatformError::InvalidPublicKeyBlob);
        }

        let magic = u32::from_le_bytes(blob[0..4].try_into().unwrap());
        let key_len = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        if magic != BCRYPT_ECDSA_PUBLIC_P256_MAGIC || key_len != 32 {
            return Err(WindowsPlatformError::InvalidPublicKeyBlob);
        }

        let mut sec1 = [0_u8; 65];
        sec1[0] = 0x04;
        sec1[1..33].copy_from_slice(&blob[8..40]);
        sec1[33..65].copy_from_slice(&blob[40..72]);
        VerifyingKey::from_sec1_bytes(&sec1)
            .map_err(|_| WindowsPlatformError::InvalidPublicKeyBlob)?;
        Ok(sec1)
    }

    pub fn device_key_id(&self) -> Result<[u8; DEVICE_KEY_ID_LEN], WindowsPlatformError> {
        let public = self.public_key_sec1()?;
        Ok(Sha256::digest(public).into())
    }

    fn sign_domain_message(
        &self,
        domain: &[u8],
        payload: &[u8],
    ) -> Result<[u8; 64], WindowsPlatformError> {
        let mut message = Vec::with_capacity(domain.len() + payload.len());
        message.extend_from_slice(domain);
        message.extend_from_slice(payload);
        let digest = Sha256::digest(&message);

        let mut required = 0_u32;
        let status = unsafe {
            NCryptSignHash(
                self.handles.key,
                null(),
                digest.as_ptr(),
                digest.len() as u32,
                null_mut(),
                0,
                &mut required,
                NCRYPT_SILENT_FLAG,
            )
        };
        cng_ok("NCryptSignHash(size)", status)?;
        if required != 64 {
            return Err(WindowsPlatformError::InvalidSignature);
        }

        let mut signature = [0_u8; 64];
        let mut written = 0_u32;
        let status = unsafe {
            NCryptSignHash(
                self.handles.key,
                null(),
                digest.as_ptr(),
                digest.len() as u32,
                signature.as_mut_ptr(),
                signature.len() as u32,
                &mut written,
                NCRYPT_SILENT_FLAG,
            )
        };
        cng_ok("NCryptSignHash", status)?;
        if written != signature.len() as u32 {
            return Err(WindowsPlatformError::InvalidSignature);
        }
        Ok(signature)
    }

    pub fn assert_possession(&self, challenge: &[u8]) -> Result<(), WindowsPlatformError> {
        let signature = self.sign_domain_message(POSSESSION_DOMAIN, challenge)?;
        let mut message = Vec::with_capacity(POSSESSION_DOMAIN.len() + challenge.len());
        message.extend_from_slice(POSSESSION_DOMAIN);
        message.extend_from_slice(challenge);

        let public = self.public_key_sec1()?;
        let verifying_key = VerifyingKey::from_sec1_bytes(&public)
            .map_err(|_| WindowsPlatformError::InvalidPublicKeyBlob)?;
        let signature =
            Signature::from_slice(&signature).map_err(|_| WindowsPlatformError::InvalidSignature)?;
        verifying_key
            .verify(&message, &signature)
            .map_err(|_| WindowsPlatformError::PossessionProofFailed)
    }

    pub fn create_activation_request(
        &self,
        request_id: &str,
        product_id: &str,
    ) -> Result<Vec<u8>, WindowsPlatformError> {
        let public = self.public_key_sec1()?;
        let device_key_id = self.device_key_id()?;
        let facts = ActivationRequestFactsV1 {
            schema_version: 1,
            request_id: request_id.to_owned(),
            product_id: product_id.to_owned(),
            device_key_id,
            device_public_key_sec1: public.to_vec(),
        };
        let signed_facts = encode_activation_request_facts(&facts)
            .map_err(|error| WindowsPlatformError::ActivationRequest(error.to_string()))?;
        let signature =
            self.sign_domain_message(ACTIVATION_REQUEST_SIGNATURE_DOMAIN, &signed_facts)?;
        assemble_activation_request(signed_facts, signature)
            .map_err(|error| WindowsPlatformError::ActivationRequest(error.to_string()))
    }

    #[cfg(test)]
    fn delete_for_test(mut self) -> Result<(), WindowsPlatformError> {
        let status = unsafe {
            windows_sys::Win32::Security::Cryptography::NCryptDeleteKey(
                self.handles.key,
                NCRYPT_SILENT_FLAG,
            )
        };
        cng_ok("NCryptDeleteKey", status)?;
        self.handles.key = 0;
        Ok(())
    }
}

fn try_open_existing(
    provider_name: PCWSTR,
    backing: DeviceKeyBacking,
    key_name: &[u16],
) -> Result<Option<WindowsDeviceKey>, WindowsPlatformError> {
    let provider =
        open_provider(provider_name).map_err(|status| cng_error("NCryptOpenStorageProvider", status))?;

    let mut key = 0;
    let status = unsafe {
        NCryptOpenKey(
            provider,
            &mut key,
            key_name.as_ptr(),
            0,
            NCRYPT_SILENT_FLAG,
        )
    };
    if status == 0 {
        return Ok(Some(WindowsDeviceKey {
            handles: KeyHandles { provider, key },
            backing,
        }));
    }

    unsafe {
        let _ = NCryptFreeObject(provider);
    }
    if status == NTE_BAD_KEYSET {
        Ok(None)
    } else {
        Err(cng_error("NCryptOpenKey", status))
    }
}

fn create_with_provider(
    provider_name: PCWSTR,
    backing: DeviceKeyBacking,
    key_name: &[u16],
) -> Result<WindowsDeviceKey, WindowsPlatformError> {
    let provider =
        open_provider(provider_name).map_err(|status| cng_error("NCryptOpenStorageProvider", status))?;
    let mut key = 0;
    let status = unsafe {
        NCryptCreatePersistedKey(
            provider,
            &mut key,
            BCRYPT_ECDSA_P256_ALGORITHM,
            key_name.as_ptr(),
            0,
            NCRYPT_SILENT_FLAG,
        )
    };

    if status == NTE_EXISTS {
        unsafe {
            let _ = NCryptFreeObject(provider);
        }
        return try_open_existing(provider_name, backing, key_name)?
            .ok_or_else(|| cng_error("NCryptCreatePersistedKey/race", status));
    }

    if status != 0 {
        unsafe {
            let _ = NCryptFreeObject(provider);
        }
        return Err(cng_error("NCryptCreatePersistedKey", status));
    }

    // CNG defines export policy value 0 as non-exportable. Set it explicitly
    // before finalization rather than relying only on the provider default.
    let export_policy = 0_u32;
    let export_policy_name = wide("Export Policy");
    let set_policy = unsafe {
        NCryptSetProperty(
            key,
            export_policy_name.as_ptr(),
            (&export_policy as *const u32).cast::<u8>(),
            std::mem::size_of::<u32>() as u32,
            0,
        )
    };
    if set_policy != 0 {
        unsafe {
            let _ = NCryptFreeObject(key);
            let _ = NCryptFreeObject(provider);
        }
        return Err(cng_error("NCryptSetProperty(Export Policy)", set_policy));
    }

    let finalize = unsafe { NCryptFinalizeKey(key, NCRYPT_SILENT_FLAG) };
    if finalize != 0 {
        unsafe {
            let _ = NCryptFreeObject(key);
            let _ = NCryptFreeObject(provider);
        }
        return Err(cng_error("NCryptFinalizeKey", finalize));
    }

    Ok(WindowsDeviceKey {
        handles: KeyHandles { provider, key },
        backing,
    })
}

fn open_provider(provider_name: PCWSTR) -> Result<NCRYPT_PROV_HANDLE, i32> {
    let mut provider = 0;
    let status = unsafe { NCryptOpenStorageProvider(&mut provider, provider_name, 0) };
    if status == 0 {
        Ok(provider)
    } else {
        Err(status)
    }
}

fn is_bootstrap_tpm_unavailable(error: &WindowsPlatformError) -> bool {
    matches!(
        error,
        WindowsPlatformError::Cng { status, .. }
            if matches!(
                *status,
                NTE_NOT_SUPPORTED_STATUS
                    | NTE_DEVICE_NOT_READY_STATUS
                    | NTE_DEVICE_NOT_FOUND_STATUS
            )
    )
}

fn cng_ok(api: &'static str, status: i32) -> Result<(), WindowsPlatformError> {
    if status == 0 {
        Ok(())
    } else {
        Err(cng_error(api, status))
    }
}

fn cng_error(api: &'static str, status: i32) -> WindowsPlatformError {
    WindowsPlatformError::Cng {
        api,
        status: status as u32,
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

struct TrustedTimeMutexGuard {
    handle: HANDLE,
    owned: bool,
}

impl TrustedTimeMutexGuard {
    fn acquire(
        path: &Path,
        device_key_id: &[u8; DEVICE_KEY_ID_LEN],
    ) -> Result<Self, WindowsPlatformError> {
        let name = trusted_time_mutex_name(path, device_key_id)?;
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("CreateMutexW"));
        }

        let wait = unsafe { WaitForSingleObject(handle, STATE_MUTEX_WAIT_MS) };
        match wait {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self {
                handle,
                owned: true,
            }),
            WAIT_TIMEOUT => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(WindowsPlatformError::StateLockTimeout)
            }
            WAIT_FAILED => {
                let error = last_error("WaitForSingleObject");
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(error)
            }
            _ => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(last_error("WaitForSingleObject(unexpected)"))
            }
        }
    }
}

impl Drop for TrustedTimeMutexGuard {
    fn drop(&mut self) {
        unsafe {
            if self.owned {
                let _ = ReleaseMutex(self.handle);
            }
            let _ = CloseHandle(self.handle);
        }
    }
}

fn trusted_time_mutex_name(
    path: &Path,
    device_key_id: &[u8; DEVICE_KEY_ID_LEN],
) -> Result<Vec<u16>, WindowsPlatformError> {
    let parent = path.parent().ok_or(WindowsPlatformError::InvalidStatePath)?;
    let file_name = path.file_name().ok_or(WindowsPlatformError::InvalidStatePath)?;
    fs::create_dir_all(parent)?;
    let canonical_parent = fs::canonicalize(parent)?;
    let canonical_path = canonical_parent.join(file_name);

    let mut hasher = Sha256::new();
    hasher.update(STATE_MUTEX_DOMAIN);
    for unit in canonical_path.as_os_str().encode_wide() {
        hasher.update(unit.to_le_bytes());
    }
    hasher.update(device_key_id);
    let digest = hasher.finalize();

    let mut name = String::from("Local\\Chaptera.TrustedTimeState.");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut name, "{byte:02x}").expect("write mutex hash");
    }
    Ok(wide(&name))
}

struct OutBlob(CRYPT_INTEGER_BLOB);

impl Default for OutBlob {
    fn default() -> Self {
        Self(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        })
    }
}

impl Drop for OutBlob {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            unsafe {
                std::ptr::write_bytes(self.0.pbData, 0, self.0.cbData as usize);
                let _ = LocalFree(self.0.pbData.cast::<c_void>());
            }
        }
    }
}

fn dpapi_entropy(device_key_id: &[u8; DEVICE_KEY_ID_LEN]) -> Vec<u8> {
    let mut entropy = Vec::with_capacity(STATE_ENTROPY_DOMAIN.len() + DEVICE_KEY_ID_LEN);
    entropy.extend_from_slice(STATE_ENTROPY_DOMAIN);
    entropy.extend_from_slice(device_key_id);
    entropy
}

fn protect_current_user(
    plaintext: &[u8],
    device_key_id: &[u8; DEVICE_KEY_ID_LEN],
) -> Result<Vec<u8>, WindowsPlatformError> {
    let entropy = dpapi_entropy(device_key_id);
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plaintext.len())
            .map_err(|_| WindowsPlatformError::StateEncode("state too large".into()))?,
        pbData: plaintext.as_ptr().cast_mut(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr().cast_mut(),
    };
    let mut output = OutBlob::default();

    let ok = unsafe {
        CryptProtectData(
            &input,
            null(),
            &entropy_blob,
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };
    if ok == 0 {
        return Err(last_error("CryptProtectData"));
    }
    if output.0.pbData.is_null() {
        return Err(last_error("CryptProtectData(null output)"));
    }

    Ok(unsafe {
        std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize).to_vec()
    })
}

fn unprotect_current_user(
    ciphertext: &[u8],
    device_key_id: &[u8; DEVICE_KEY_ID_LEN],
) -> Result<Vec<u8>, WindowsPlatformError> {
    let entropy = dpapi_entropy(device_key_id);
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(ciphertext.len())
            .map_err(|_| WindowsPlatformError::StateDecode("state too large".into()))?,
        pbData: ciphertext.as_ptr().cast_mut(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr().cast_mut(),
    };
    let mut output = OutBlob::default();

    let ok = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            &entropy_blob,
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };
    if ok == 0 {
        return Err(last_error("CryptUnprotectData"));
    }
    if output.0.pbData.is_null() {
        return Err(last_error("CryptUnprotectData(null output)"));
    }

    Ok(unsafe {
        std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize).to_vec()
    })
}

fn last_error(api: &'static str) -> WindowsPlatformError {
    WindowsPlatformError::Win32 {
        api,
        code: unsafe { GetLastError() },
    }
}

pub fn save_trusted_time_state(
    path: &Path,
    state: &TrustedTimeStateV1,
) -> Result<(), WindowsPlatformError> {
    let mut plaintext = Vec::new();
    ciborium::ser::into_writer(state, &mut plaintext)
        .map_err(|error| WindowsPlatformError::StateEncode(error.to_string()))?;
    let protected = protect_current_user(&plaintext, &state.device_key_id)?;

    let parent = path.parent().ok_or(WindowsPlatformError::InvalidStatePath)?;
    fs::create_dir_all(parent)?;
    let temp = unique_temp_path(path)?;

    let write_result = (|| -> Result<(), WindowsPlatformError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&protected)?;
        file.sync_all()?;
        drop(file);

        let source = wide_path(&temp);
        let destination = wide_path(path);
        let moved = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            return Err(last_error("MoveFileExW"));
        }
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

pub fn evaluate_trusted_time_transaction(
    path: &Path,
    expected_device_key_id: &[u8; DEVICE_KEY_ID_LEN],
    now: i64,
    lease: &LeaseTimeInputV1<'_>,
    policy: TimePolicy,
) -> Result<TimeAcceptance, WindowsPlatformError> {
    let _guard = TrustedTimeMutexGuard::acquire(path, expected_device_key_id)?;
    let state = load_trusted_time_state(path, expected_device_key_id)?
        .unwrap_or_else(|| TrustedTimeStateV1::fresh(*expected_device_key_id));

    let accepted =
        evaluate_time_bound_right(&state, expected_device_key_id, now, lease, policy)?;
    save_trusted_time_state(path, accepted.next_state())?;
    Ok(accepted)
}

pub fn load_trusted_time_state(
    path: &Path,
    expected_device_key_id: &[u8; DEVICE_KEY_ID_LEN],
) -> Result<Option<TrustedTimeStateV1>, WindowsPlatformError> {
    let ciphertext = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };

    let plaintext = unprotect_current_user(&ciphertext, expected_device_key_id)?;
    let state: TrustedTimeStateV1 = ciborium::de::from_reader(Cursor::new(plaintext))
        .map_err(|error| WindowsPlatformError::StateDecode(error.to_string()))?;
    if &state.device_key_id != expected_device_key_id {
        return Err(WindowsPlatformError::StateDeviceMismatch);
    }
    Ok(Some(state))
}

fn unique_temp_path(path: &Path) -> Result<PathBuf, WindowsPlatformError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(WindowsPlatformError::InvalidStatePath)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(path.with_file_name(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        sequence
    )))
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos()
    }

    const TXN_CHILD_ENV: &str = "CHAPTERA_ENTITLEMENT_TXN_CHILD";
    const TXN_PATH_ENV: &str = "CHAPTERA_ENTITLEMENT_TXN_PATH";
    const TXN_DEVICE: [u8; DEVICE_KEY_ID_LEN] = [0x4C; DEVICE_KEY_ID_LEN];

    fn txn_policy() -> TimePolicy {
        TimePolicy {
            skew_seconds: 0,
            forward_jump_limit_seconds: None,
        }
    }

    fn txn_lease() -> LeaseTimeInputV1<'static> {
        LeaseTimeInputV1 {
            lease_id: "txn-lease-1",
            lease_sequence: 1,
            issued_at: 1_900_000_000,
            not_before: 1_900_000_000,
            valid_until: 1_900_100_000,
            offline_grace_until: Some(1_900_200_000),
            authenticated_server_time: Some(1_900_000_000),
            lease_commitment: [0x44; crate::LEASE_COMMITMENT_LEN],
        }
    }

    #[test]
    fn trusted_time_transaction_child() {
        let Some(mode) = std::env::var_os(TXN_CHILD_ENV) else {
            return;
        };
        let path = PathBuf::from(
            std::env::var_os(TXN_PATH_ENV).expect("transaction child path"),
        );

        if mode == "abandon" {
            let _guard =
                TrustedTimeMutexGuard::acquire(&path, &TXN_DEVICE).expect("child mutex acquire");
            std::process::exit(0);
        }

        evaluate_trusted_time_transaction(
            &path,
            &TXN_DEVICE,
            1_900_000_100,
            &txn_lease(),
            txn_policy(),
        )
        .expect("child transaction");
    }

    #[test]
    fn cross_process_transactions_serialize_without_lost_generations() {
        if std::env::var_os(TXN_CHILD_ENV).is_some() {
            return;
        }

        let dir = std::env::temp_dir().join(format!(
            "chaptera-entitlement-txn-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let path = dir.join("trusted-time-v1.bin");
        let exe = std::env::current_exe().expect("current test executable");

        let mut children = Vec::new();
        for _ in 0..8 {
            let child = Command::new(&exe)
                .arg("--exact")
                .arg("windows_platform::tests::trusted_time_transaction_child")
                .arg("--nocapture")
                .env(TXN_CHILD_ENV, "commit")
                .env(TXN_PATH_ENV, &path)
                .spawn()
                .expect("spawn transaction child");
            children.push(child);
        }

        for mut child in children {
            let status = child.wait().expect("wait for transaction child");
            assert!(status.success(), "transaction child failed: {status}");
        }

        let final_state = load_trusted_time_state(&path, &TXN_DEVICE)
            .expect("load final transaction state")
            .expect("transaction state exists");
        assert_eq!(final_state.generation, 8);
        assert_eq!(final_state.highest_lease_sequence, Some(1));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejected_transaction_does_not_persist_candidate_state() {
        let dir = std::env::temp_dir().join(format!(
            "chaptera-entitlement-txn-reject-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let path = dir.join("trusted-time-v1.bin");

        evaluate_trusted_time_transaction(
            &path,
            &TXN_DEVICE,
            1_900_000_100,
            &txn_lease(),
            txn_policy(),
        )
        .expect("first accepted transaction");

        let mut replay = txn_lease();
        replay.lease_sequence = 0;
        let error = evaluate_trusted_time_transaction(
            &path,
            &TXN_DEVICE,
            1_900_000_101,
            &replay,
            txn_policy(),
        )
        .expect_err("invalid replay must fail");
        assert!(matches!(
            error,
            WindowsPlatformError::TrustedTime(TrustedTimeError::InvalidLease)
        ));

        let final_state = load_trusted_time_state(&path, &TXN_DEVICE)
            .expect("load after rejected transaction")
            .expect("state exists");
        assert_eq!(final_state.generation, 1);
        assert_eq!(final_state.highest_lease_sequence, Some(1));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn abandoned_cross_process_mutex_is_recovered() {
        if std::env::var_os(TXN_CHILD_ENV).is_some() {
            return;
        }

        let dir = std::env::temp_dir().join(format!(
            "chaptera-entitlement-txn-abandon-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let path = dir.join("trusted-time-v1.bin");
        let exe = std::env::current_exe().expect("current test executable");

        let status = Command::new(&exe)
            .arg("--exact")
            .arg("windows_platform::tests::trusted_time_transaction_child")
            .arg("--nocapture")
            .env(TXN_CHILD_ENV, "abandon")
            .env(TXN_PATH_ENV, &path)
            .status()
            .expect("spawn abandoned mutex child");
        assert!(status.success());

        evaluate_trusted_time_transaction(
            &path,
            &TXN_DEVICE,
            1_900_000_100,
            &txn_lease(),
            txn_policy(),
        )
        .expect("recover abandoned mutex and commit");

        let final_state = load_trusted_time_state(&path, &TXN_DEVICE)
            .expect("load recovered state")
            .expect("recovered state exists");
        assert_eq!(final_state.generation, 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persisted_device_key_reopens_and_proves_possession() {
        let key_name = format!(
            "Chaptera.DeviceKey.slice-c.test.{}.{}",
            std::process::id(),
            unique_suffix()
        );

        let first = WindowsDeviceKey::open_or_create(&key_name).expect("create/open DeviceKey");
        let backing = first.backing();
        let first_id = first.device_key_id().expect("derive DeviceKeyId");
        first
            .assert_possession(b"slice-c-first-possession")
            .expect("CNG possession proof");
        drop(first);

        let second = WindowsDeviceKey::open_existing(&key_name, backing)
            .expect("reopen DeviceKey from pinned backing");
        let second_id = second.device_key_id().expect("derive reopened DeviceKeyId");
        assert_eq!(first_id, second_id);
        assert_eq!(backing, second.backing());
        second
            .assert_possession(b"slice-c-second-possession")
            .expect("reopened CNG possession proof");

        eprintln!("DeviceKey backing={backing:?} id={}", hex(&second_id));
        second.delete_for_test().expect("delete test DeviceKey");
    }

    #[test]
    fn dpapi_state_round_trip_atomic_replace_and_corruption_fail_closed() {
        let device_id = [0xA5; DEVICE_KEY_ID_LEN];
        let mut state = TrustedTimeStateV1::fresh(device_id);
        state.generation = 7;
        state.local_wall_high_water = Some(1_900_000_000);
        state.server_time_high_water = Some(1_899_999_000);
        state.highest_lease_sequence = Some(42);
        state.last_lease_id = Some("lease-42".into());

        let dir = std::env::temp_dir().join(format!(
            "chaptera-entitlement-slice-c-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let path = dir.join("trusted-time-v1.bin");

        save_trusted_time_state(&path, &state).expect("initial protected state write");
        let loaded = load_trusted_time_state(&path, &device_id)
            .expect("load protected state")
            .expect("state exists");
        assert_eq!(loaded, state);

        state.generation = 8;
        state.local_wall_high_water = Some(1_900_000_100);
        save_trusted_time_state(&path, &state).expect("atomic replacement");
        let loaded = load_trusted_time_state(&path, &device_id)
            .expect("load replaced protected state")
            .expect("state exists");
        assert_eq!(loaded, state);

        let mut corrupted = fs::read(&path).expect("read protected state");
        let index = corrupted.len() / 2;
        corrupted[index] ^= 0x80;
        fs::write(&path, corrupted).expect("write corrupted fixture");
        assert!(
            load_trusted_time_state(&path, &device_id).is_err(),
            "tampered DPAPI state must fail closed"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dpapi_entropy_binds_state_to_device_key_id() {
        let device_a = [0x11; DEVICE_KEY_ID_LEN];
        let device_b = [0x22; DEVICE_KEY_ID_LEN];
        let state = TrustedTimeStateV1::fresh(device_a);

        let dir = std::env::temp_dir().join(format!(
            "chaptera-entitlement-slice-c-entropy-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let path = dir.join("trusted-time-v1.bin");
        save_trusted_time_state(&path, &state).expect("protected state write");

        assert!(
            load_trusted_time_state(&path, &device_b).is_err(),
            "wrong DeviceKey-bound entropy must fail closed"
        );

        let _ = fs::remove_dir_all(dir);
    }

    fn hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut out, "{byte:02x}").expect("write to string");
        }
        out
    }
}
