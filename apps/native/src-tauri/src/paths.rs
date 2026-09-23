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

/// `~/.marvis/models/whisper` — path only; callers create it when needed.
#[allow(dead_code)]
pub fn whisper_dir() -> PathBuf {
    models_dir().join("whisper")
}

/// `~/.marvis/models/whisper/bin` — path only; callers create it when needed.
#[allow(dead_code)]
pub fn whisper_bin_dir() -> PathBuf {
    whisper_dir().join("bin")
}

/// Return the architecture-specific Whisper CLI staged beside the application
/// executable by Tauri's `externalBin` bundle layout (`Contents/MacOS` on macOS).
/// Packaging stages exactly this target-triple name, so discovery cannot select a
/// neighboring architecture's binary. This helper remains platform-neutral and
/// does not inspect the filesystem.
#[allow(dead_code)] // Used by Task 4 after resolving the executable directory.
pub fn bundled_whisper_cli(executable_dir: &std::path::Path) -> PathBuf {
    executable_dir.join(format!("whisper-cli-{}", whisper_target_triple()))
}

/// Return the sidecar name emitted by Tauri inside a packaged macOS app.
/// Tauri consumes the target-suffixed staging file and strips that suffix when
/// it copies the external binary beside the application executable.
#[allow(dead_code)]
pub fn packaged_whisper_cli(executable_dir: &std::path::Path) -> PathBuf {
    executable_dir.join("whisper-cli")
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
const fn whisper_target_triple() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "unknown-apple-darwin"
    }
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
const fn whisper_target_triple() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "unknown-unknown-linux-gnu"
    }
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
const fn whisper_target_triple() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-pc-windows-msvc"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "unknown-pc-windows-msvc"
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
#[allow(dead_code)]
const fn whisper_target_triple() -> &'static str {
    "unknown-unknown-unknown"
}

/// `~/.marvis/models/whisper/models` — path only; callers create it when needed.
#[allow(dead_code)]
pub fn whisper_models_dir() -> PathBuf {
    whisper_dir().join("models")
}

/// `~/.marvis/tmp` — callers create it when they need it.
#[allow(dead_code)]
pub fn audio_tmp_dir() -> PathBuf {
    root().join("tmp")
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

        let whisper = root.join("models").join("whisper");
        let bin = whisper.join("bin");
        let models = whisper.join("models");
        let tmp = root.join("tmp");
        let existed = [
            whisper.exists(),
            bin.exists(),
            models.exists(),
            tmp.exists(),
        ];
        assert_eq!(whisper_dir(), whisper);
        assert_eq!(whisper_bin_dir(), bin);
        assert_eq!(whisper_models_dir(), models);
        assert_eq!(audio_tmp_dir(), tmp);
        assert_eq!(
            [
                whisper.exists(),
                bin.exists(),
                models.exists(),
                tmp.exists()
            ],
            existed
        );
    }

    #[test]
    fn bundled_whisper_cli_is_relative_to_external_bin_dir_and_target_specific() {
        let executable_dir = PathBuf::from("/app/Contents/MacOS");
        let candidate = bundled_whisper_cli(&executable_dir);
        assert_eq!(candidate.parent(), Some(executable_dir.as_path()));
        assert_eq!(
            candidate.file_name().unwrap().to_string_lossy(),
            format!("whisper-cli-{}", whisper_target_triple())
        );
        assert_eq!(
            packaged_whisper_cli(&executable_dir),
            executable_dir.join("whisper-cli")
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_has_0700_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
