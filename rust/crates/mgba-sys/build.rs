//! Builds libmgba + csrc/pd_core.c with CMake (Ninja + the MinGW GCC on PATH) and links them in.

fn main() {
    let dst = cmake::Config::new(".")
        .generator("Ninja")
        .profile("Release")
        .build_target("pd_core")
        .build();
    let build = dst.join("build");
    println!("cargo:rustc-link-search=native={}", build.display());
    println!(
        "cargo:rustc-link-search=native={}",
        build.join("mgba").display()
    );
    println!("cargo:rustc-link-lib=static=pd_core");
    println!("cargo:rustc-link-lib=static=mgba");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        for lib in ["ws2_32", "shlwapi", "ole32", "shell32", "uuid"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    } else {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-changed=CMakeLists.txt");
}
