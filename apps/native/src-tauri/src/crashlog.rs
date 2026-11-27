/// Last-resort crash surfacing. `panic = "abort"` plus
/// `windows_subsystem = "windows"` means a startup panic or a `.setup`
/// `?` failure exits in TOTAL silence — nothing on screen, nothing on
/// any console. Panic/`build` failures therefore land in
/// `~/.marvis/crash.log` and, on Windows, a native message box that
/// needs no webview. Deliberately bypasses `paths::root()` — this path
/// must never panic when HOME is missing (that's itself a candidate
/// crash).
pub(crate) fn report_fatal(detail: &str) {
    let path = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".marvis")
        .join("crash.log");
    if let Some(parent) = path.parent() {
        // keys.json/config live under this root — create it at 0700 like
        // `paths::root()` does (a first-run crash may beat it there).
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        let _ = builder.create(parent);
    }
    let _ = std::fs::write(&path, format!("marvis failed to start: {detail}\n"));
    #[cfg(target_os = "windows")]
    {
        rfd::MessageDialog::new()
            .set_title("Marvis failed to start")
            .set_description(format!("{detail}\n\nDetails: {}", path.display()))
            .set_level(rfd::MessageLevel::Error)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}
