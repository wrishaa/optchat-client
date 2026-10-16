//! Linux hooks: Secret Service tokens, /proc resident memory, and a test QR hook.
use gpui::{App, Window};
use optchat_platform::{Platform, ScanDone, SecureStore};
use std::sync::Arc;

const SERVICE: &str = "com.reframe.optchat.gpui";
const LABEL: &str = "OptChat client token";

struct SecretService;

impl SecureStore for SecretService {
    fn get(&self, key: &str) -> Option<String> {
        smol::block_on(async {
            let ring = oo7::Keyring::new().await.ok()?;
            ring.unlock().await.ok()?;
            let items = ring
                .search_items(&[("service", SERVICE), ("account", key)])
                .await
                .ok()?;
            let item = items.into_iter().next()?;
            item.unlock().await.ok()?;
            String::from_utf8(item.secret().await.ok()?.to_vec()).ok()
        })
    }

    fn set(&self, key: &str, token: &str) -> Result<(), String> {
        // Errors describe the service failure, never the token.
        smol::block_on(async {
            let ring = oo7::Keyring::new().await?;
            ring.unlock().await?;
            ring.create_item(
                LABEL,
                &[("service", SERVICE), ("account", key)],
                token.as_bytes(),
                true,
            )
            .await?;
            Ok::<_, oo7::Error>(())
        })
        .map_err(|e| format!("Secret Service: {e}"))
    }

    fn delete(&self, key: &str) {
        let _ = smol::block_on(async {
            let ring = oo7::Keyring::new().await?;
            ring.unlock().await?;
            for item in ring
                .search_items(&[("service", SERVICE), ("account", key)])
                .await?
            {
                item.delete().await?;
            }
            Ok::<_, oo7::Error>(())
        });
    }
}

pub struct Linux {
    store: Arc<dyn SecureStore>,
    test_qr: Option<String>,
}

impl Linux {
    pub fn new() -> Self {
        Self {
            store: Arc::new(SecretService),
            test_qr: std::env::var("OPTCHAT_TEST_QR").ok(),
        }
    }
}

impl Platform for Linux {
    fn secure_store(&self) -> Option<Arc<dyn SecureStore>> {
        Some(self.store.clone())
    }

    fn resident_mb(&self) -> Option<f64> {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        (page_size > 0).then(|| pages as f64 * page_size as f64 / (1024.0 * 1024.0))
    }

    fn can_scan_qr(&self) -> bool {
        self.test_qr.is_some()
    }

    fn scan_qr(&self, window: &mut Window, cx: &mut App, done: ScanDone) {
        done(self.test_qr.clone(), window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_memory_is_available() {
        assert!(Linux::new().resident_mb().is_some_and(|mb| mb > 0.0));
    }

    /// An opt-in test creates and removes only its own temporary keyring entry.
    #[test]
    #[ignore = "requires an unlocked desktop Secret Service"]
    fn secret_service_round_trip() {
        let store = SecretService;
        let key = format!("test|linux-roundtrip-{}", std::process::id());
        store.set(&key, "not-a-real-token").unwrap();
        assert_eq!(store.get(&key).as_deref(), Some("not-a-real-token"));
        store.set(&key, "updated-test-token").unwrap();
        assert_eq!(store.get(&key).as_deref(), Some("updated-test-token"));
        store.delete(&key);
        assert_eq!(store.get(&key), None);
    }
}
