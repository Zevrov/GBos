//! The exe's icon (Explorer, the taskbar, a download): assets/icon.ico (rust/tools/make_icon.py) in a Windows
//! resource, compiled with MinGW's windres (the toolchain the build already needs) and linked into the binary.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = dir.join("assets").join("icon.ico");
    let rc = out.join("icon.rc");
    let obj = out.join("icon.o");
    std::fs::write(
        &rc,
        format!(
            "1 ICON \"{}\"\n",
            ico.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    match Command::new("windres")
        .arg("-i")
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .status()
    {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", obj.display()),
        // No windres: the app builds all the same, with Windows' plain icon.
        _ => println!("cargo:warning=windres not found: AdventurePad.exe gets no icon"),
    }
}
