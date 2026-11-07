//! Linux: the executable's RUNPATH is its own directory, so `dlopen("libopenxr_loader.so")`
//! finds an OpenXR loader shipped beside it (SteamOS and the Steam Frame have none) before
//! looking in the system directories.
//! Windows: links in the executable's icon, manifest and version (packaging/windows).

use std::path::Path;

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
    // The icon in neon-lasertag.exe, for Explorer, shortcuts and the taskbar.
    if os == "windows" {
        println!("cargo::rerun-if-changed=packaging/windows/lasertag.rc");
        println!("cargo::rerun-if-changed=packaging/windows/lasertag.ico");
        println!("cargo::rerun-if-changed=packaging/windows/lasertag.manifest");
        let out_dir = std::env::var("OUT_DIR").unwrap();
        version_resource(Path::new(&out_dir));
        embed_resource::compile_for(
            "packaging/windows/lasertag.rc",
            ["neon-lasertag"],
            embed_resource::ParamsIncludeDirs([&out_dir]),
        )
        .manifest_required()
        .unwrap();
    }
}

/// Writes version.rc into `out_dir` for packaging/windows/lasertag.rc to include: the version
/// and names that Explorer shows on the Details tab of the executable's properties.
fn version_resource(out_dir: &Path) {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let numbers = format!(
        "{},{},{},0",
        var("CARGO_PKG_VERSION_MAJOR"),
        var("CARGO_PKG_VERSION_MINOR"),
        var("CARGO_PKG_VERSION_PATCH")
    );
    let version = var("CARGO_PKG_VERSION");
    let strings = [
        ("CompanyName", "dividebysandwich"),
        ("FileDescription", "Neon Lasertag"),
        ("FileVersion", &version),
        ("InternalName", "neon-lasertag"),
        ("OriginalFilename", "neon-lasertag.exe"),
        ("ProductName", "Neon Lasertag"),
        ("ProductVersion", &version),
    ];
    // An application (FILETYPE 1) for 32-bit Windows (FILEOS 0x40004).
    let mut rc = format!(
        "1 VERSIONINFO\nFILEVERSION {numbers}\nPRODUCTVERSION {numbers}\nFILEFLAGSMASK 0x3F\nFILEFLAGS 0\nFILEOS 0x40004\nFILETYPE 1\nFILESUBTYPE 0\nBEGIN\n  BLOCK \"StringFileInfo\"\n  BEGIN\n    BLOCK \"040904B0\"\n    BEGIN\n"
    );
    for (key, value) in strings {
        rc += &format!("      VALUE \"{key}\", \"{value}\"\n");
    }
    // US English, Unicode.
    rc += "    END\n  END\n  BLOCK \"VarFileInfo\"\n  BEGIN\n    VALUE \"Translation\", 0x409, 1200\n  END\nEND\n";
    std::fs::write(out_dir.join("version.rc"), rc).unwrap();
}
