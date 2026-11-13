//! Links what our static SDL links (the `vendored-sdl` feature, docs/sdl-vendoring.md).
//!
//! sdl3-sys links libSDL3.a from the sdl3.pc it installs, but reads only that file's Libs. SDL lists
//! the libraries it links directly (SDL_DEPS_SHARED=OFF: Wayland, EGL, xkbcommon) as
//! Requires.private, which takes pkg-config to resolve.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_VENDORED_SDL").is_none() {
        return;
    }
    let out = std::env::var("DEP_SDL3_OUT_DIR").expect("sdl3-sys did not report where it built SDL");
    let pc_dir = ["lib/pkgconfig", "lib64/pkgconfig"]
        .iter()
        .map(|d| Path::new(&out).join(d))
        .find(|d| d.join("sdl3.pc").is_file())
        .expect("no sdl3.pc in sdl3-sys's OUT_DIR");
    let o = Command::new("pkg-config")
        .env("PKG_CONFIG_PATH", &pc_dir)
        .args(["--static", "--libs", "sdl3"])
        .output()
        .expect("pkg-config is needed to link the vendored SDL");
    assert!(o.status.success(), "pkg-config --static --libs sdl3: {}", String::from_utf8_lossy(&o.stderr));
    for tok in String::from_utf8_lossy(&o.stdout).split_whitespace() {
        if let Some(lib) = tok.strip_prefix("-l") {
            // sdl3-sys links SDL3 itself; m/pthread/rt come with Rust's std.
            if !matches!(lib, "SDL3" | "m" | "pthread" | "rt") {
                println!("cargo::rustc-link-lib=dylib={lib}");
            }
        } else if let Some(dir) = tok.strip_prefix("-L") {
            println!("cargo::rustc-link-search=native={dir}");
        }
    }
}
