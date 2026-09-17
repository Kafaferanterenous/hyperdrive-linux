//! HyperDrive - build script: link libopenmpt when available.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    match std::process::Command::new("pkg-config")
        .args(["--libs", "libopenmpt"])
        .output()
    {
        Ok(o) if o.status.success() => {
            for tok in String::from_utf8_lossy(&o.stdout).split_whitespace() {
                if let Some(lib) = tok.strip_prefix("-l") {
                    println!("cargo:rustc-link-lib={lib}");
                }
            }
        }
        _ => {
            // Fallback: standard name.
            println!("cargo:rustc-link-lib=openmpt");
        }
    }
}
