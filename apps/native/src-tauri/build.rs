use std::process::Command;

fn main() {
    // The About panel's `version (build)` number is the git commit
    // count, baked in at compile time. Watch HEAD (branch switches and
    // detached-HEAD commits) plus the checked-out ref (normal commits)
    // so the count stays live; a non-git build just gets no build
    // number and the panel shows the bare version.
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if let Some(count) = git(&["rev-list", "--count", "HEAD"]).filter(|n| !n.is_empty()) {
        println!("cargo:rustc-env=MARVIS_BUILD_NUMBER={count}");
    }
    for path in [
        git(&["rev-parse", "--git-path", "HEAD"]),
        git(&["symbolic-ref", "-q", "HEAD"])
            .and_then(|reference| git(&["rev-parse", "--git-path", &reference])),
    ]
    .into_iter()
    .flatten()
    {
        println!("cargo:rerun-if-changed={path}");
    }

    // Local Rust builds and tests do not bundle the release-only external binary.
    // Keep the checked-in Tauri config strict for `tauri build`, while omitting
    // externalBin from debug cargo builds until CI stages a real artifact.
    if std::env::var_os("PROFILE").as_deref() == Some(std::ffi::OsStr::new("debug")) {
        std::env::set_var("TAURI_CONFIG", r#"{"bundle":{"externalBin":[]}}"#);
    }

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // Mirror screencapturekit's rpath args into OUR binaries: the crate's
    // `cargo:rustc-link-arg` only reaches its own build units, not the
    // top-level bin/test targets (documented Cargo limitation). Without
    // these, macOS 27 binaries abort at dyld load (no libswift_Concurrency
    // in the system dyld cache). macOS-only — as a `/Wl,…` flag it's noise
    // to link.exe (LNK4044) on Windows.
    if target_os == "macos" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        if let Ok(out) = Command::new("xcode-select").arg("-p").output() {
            if out.status.success() {
                let xcode = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!(
                    "cargo:rustc-link-arg=-Wl,-rpath,{xcode}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx"
                );
                println!(
                    "cargo:rustc-link-arg=-Wl,-rpath,{xcode}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx"
                );
            }
        }
    }

    // Windows/MSVC: embed the app manifest in EVERY linked target, not just
    // the app bins. tauri-build's own embed uses `rustc-link-arg-bins`, which
    // never reaches cargo test executables; they still import comctl32
    // v6-only symbols (`TaskDialogIndirect`, via tao/tauri dialogs), so
    // without the common-controls manifest the loader binds comctl32 5.82
    // and the test exe aborts before main: STATUS_ENTRYPOINT_NOT_FOUND.
    // `rustc-link-arg` covers bins+tests+examples; `new_without_app_manifest`
    // below keeps the app binary from getting a second manifest (LNK1123).
    // See https://github.com/tauri-apps/tauri/issues/13419
    if target_os == "windows" && target_env == "msvc" {
        let manifest = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set"),
        )
        .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }

    // Mirrors `tauri_build::build()` (which is just this with `Attributes::
    // default()`). On MSVC the app-manifest resource is disabled per the
    // comment above — it would collide with the `/MANIFESTINPUT` link-arg
    // (LNK1123) — while on every other target tauri-build's own embed
    // remains the only manifest source.
    let attributes = if target_os == "windows" && target_env == "msvc" {
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    if let Err(error) = tauri_build::try_build(attributes) {
        let error = format!("{error:#}");
        println!("{error}");
        if error.starts_with("unknown field") {
            print!("found an unknown configuration field. This usually means that you are using a CLI version that is newer than `tauri-build` and is incompatible. ");
            println!(
                "Please try updating the Rust crates by running `cargo update` in the Tauri app folder."
            );
        }
        std::process::exit(1);
    }
}
