fn main() {
    println!("cargo:rerun-if-env-changed=OPTCHAT_WINDOWS_REVISION");
    println!("cargo:rerun-if-changed=packaging/optchat.rc.in");
    println!("cargo:rerun-if-changed=packaging/assets/OptChat.ico");
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").expect("package version");
        let revision: u16 = std::env::var("OPTCHAT_WINDOWS_REVISION")
            .unwrap_or_else(|_| "0".into())
            .parse()
            .expect("OPTCHAT_WINDOWS_REVISION must be 0..65535");
        let version = format!("{version}.{revision}");
        let numeric = version.replace('.', ",");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let icon = root
            .join("packaging/assets/OptChat.ico")
            .display()
            .to_string()
            .replace('\\', "/");
        let resource = std::fs::read_to_string(root.join("packaging/optchat.rc.in"))
            .expect("resource template")
            .replace("@NUMERIC_VERSION@", &numeric)
            .replace("@VERSION@", &version)
            .replace("@ICON@", &icon);
        let path =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("optchat.rc");
        std::fs::write(&path, resource).expect("write Windows resources");
        embed_resource::compile(&path, embed_resource::NONE)
            .manifest_optional()
            .expect("embed OptChat Windows resources");
    }
}
