//! Credential Manager stores secrets for the current user; preferences never contain tokens.
use optchat_platform::{Platform, SecureStore};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use windows_sys::Win32::{
    Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE,
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    },
    System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    },
};

pub struct Windows;

impl Windows {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for Windows {
    fn secure_store(&self) -> Option<Arc<dyn SecureStore>> {
        Some(Arc::new(CredentialStore))
    }

    fn resident_mb(&self) -> Option<f64> {
        // SAFETY: the current-process pseudo-handle is valid and info has the required size.
        unsafe {
            let mut info: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            info.cb = std::mem::size_of_val(&info) as u32;
            (GetProcessMemoryInfo(GetCurrentProcess(), &mut info, info.cb) != 0)
                .then_some(info.WorkingSetSize as f64 / (1024.0 * 1024.0))
        }
    }
}

struct CredentialStore;

fn target_name(key: &str) -> Vec<u16> {
    // Engine URLs/workspace names can exceed Credential Manager's target-name limit.
    // A stable digest also keeps those names out of Credential Manager's UI.
    format!(
        "com.reframe.optchat.gpui:{:x}",
        Sha256::digest(key.as_bytes())
    )
    .encode_utf16()
    .chain(Some(0))
    .collect()
}

impl SecureStore for CredentialStore {
    fn get(&self, key: &str) -> Option<String> {
        let target = target_name(key);
        let mut credential = std::ptr::null_mut();
        // SAFETY: target is NUL-terminated; CredReadW allocates the result until CredFree.
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                return None;
            }
            let value = if (*credential).CredentialBlob.is_null() {
                None
            } else {
                String::from_utf8(
                    std::slice::from_raw_parts(
                        (*credential).CredentialBlob,
                        (*credential).CredentialBlobSize as usize,
                    )
                    .to_vec(),
                )
                .ok()
            };
            CredFree(credential.cast());
            value
        }
    }

    fn set(&self, key: &str, token: &str) -> Result<(), String> {
        if token.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            return Err("Token exceeds Windows Credential Manager's size limit".into());
        }
        let mut target = target_name(key);
        // SAFETY: all pointers reference live buffers for this synchronous call.
        unsafe {
            let mut credential: CREDENTIALW = std::mem::zeroed();
            credential.Type = CRED_TYPE_GENERIC;
            credential.TargetName = target.as_mut_ptr();
            credential.CredentialBlobSize = token.len() as u32;
            credential.CredentialBlob = token.as_ptr() as *mut u8;
            // Persists across logins for this Windows user, without roaming to other machines.
            credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
            if CredWriteW(&credential, 0) == 0 {
                return Err(format!(
                    "Windows Credential Manager: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }

    fn delete(&self, key: &str) {
        let target = target_name(key);
        // SAFETY: target is a live NUL-terminated UTF-16 buffer.
        unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_round_trip_and_size_limit() {
        let key = format!(
            "windows-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        );
        let store = CredentialStore;
        let result = store.set(&key, "test-only-秘密");
        if result.is_err() {
            store.delete(&key);
        }
        result.unwrap();
        let read = store.get(&key);
        store.delete(&key);
        assert_eq!(read.as_deref(), Some("test-only-秘密"));
        assert_eq!(store.get(&key), None);
        assert!(store
            .set(
                &key,
                &"x".repeat(CRED_MAX_CREDENTIAL_BLOB_SIZE as usize + 1)
            )
            .is_err());
        assert!(Windows.resident_mb().is_some_and(|mb| mb > 0.0));
    }
}
