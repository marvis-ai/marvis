use std::path::PathBuf;

/// `~/.marvis` — the only data root. Created with 0700 on first call.
pub fn root() -> PathBuf {
    let dir = dirs::home_dir().expect("no home dir").join(".marvis");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).expect("create ~/.marvis");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    dir
}

/// `keys.json` — plaintext provider→key map (0600 inside the 0700 root).
/// The retired `keys.enc` was an AES-GCM vault keyed by a Keychain DEK;
/// old installs may still carry one — it is left in place (unreadable
/// without the DEK, harmless) rather than deleted under the user.
pub fn keys_file() -> PathBuf {
    root().join("keys.json")
}

pub fn config_file() -> PathBuf {
    root().join("config.toml")
}

pub fn db_file() -> PathBuf {
    root().join("marvis.db")
}

/// Path only — callers create the directory when they need it.
#[allow(dead_code)] // Phase 2 models/ directory
pub fn models_dir() -> PathBuf {
    root().join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_live_under_marvis_root() {
        let root = root();
        assert!(root.ends_with(".marvis"));
        assert_eq!(keys_file(), root.join("keys.json"));
        assert_eq!(config_file(), root.join("config.toml"));
        assert_eq!(db_file(), root.join("marvis.db"));
        // models_dir() must not create the directory eagerly.
        let existed = root.join("models").exists();
        assert_eq!(models_dir(), root.join("models"));
        assert_eq!(root.join("models").exists(), existed);
    }

    #[cfg(unix)]
    #[test]
    fn root_has_0700_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
