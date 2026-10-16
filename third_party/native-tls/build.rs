#![allow(clippy::unusual_byte_groupings)]
use std::env;

fn have_min_max_version() -> bool {
    if let Ok(version) = env::var("DEP_OPENSSL_VERSION_NUMBER") {
        let version = u64::from_str_radix(&version, 16).unwrap();

        if version >= 0x1_01_00_00_0 {
            return true;
        }
    }

    if let Ok(version) = env::var("DEP_OPENSSL_LIBRESSL_VERSION_NUMBER") {
        let version = u64::from_str_radix(&version, 16).unwrap();

        if version >= 0x2_06_01_00_0 {
            return true;
        }
    }

    false
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(have_openssl_system_policy)");
    if env::var("DEP_OPENSSL_VERSION_NUMBER")
        .ok()
        .and_then(|v| u64::from_str_radix(&v, 16).ok())
        .is_some_and(|v| v >= 0x1_01_01_00_0)
    {
        println!("cargo:rustc-cfg=have_openssl_system_policy");
    }
    println!("cargo::rustc-check-cfg=cfg(have_min_max_version)");
    if have_min_max_version() {
        println!("cargo:rustc-cfg=have_min_max_version");
    }
}
