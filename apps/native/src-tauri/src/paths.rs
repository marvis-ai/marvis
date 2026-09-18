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

pub fn keys_file() -> PathBuf {
    root().join("keys.enc")
}

pub fn config_file() -> PathBuf {
    root().join("config.toml")
}

pub fn db_file() -> PathBuf {
    root().join("marvis.db")
}

/// Path only — callers create the directory when they need it.
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
        assert_eq!(keys_file(), root.join("keys.enc"));
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
