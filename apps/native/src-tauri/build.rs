use std::process::Command;

fn main() {
    // Mirror screencapturekit's rpath args into OUR binaries: the crate's
    // `cargo:rustc-link-arg` only reaches its own build units, not the
    // top-level bin/test targets (documented Cargo limitation). Without
    // these, macOS 27 binaries abort at dyld load (no libswift_Concurrency
    // in the system dyld cache).
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
    tauri_build::build()
}
