//! Linux: the executable's RUNPATH is its own directory, so `dlopen("libopenxr_loader.so")`
//! finds an OpenXR loader shipped beside it (SteamOS and the Steam Frame have none) before
//! looking in the system directories.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "linux" {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
