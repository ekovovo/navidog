//! Stamps the build channel and time into the binary.
//!
//! vergen 10 needs rustc 1.96, above this workspace's MSRV (`rust-version`
//! 1.92), so the stamp is set here. CI exports `NAVIDOG_BUILD_TIMESTAMP`
//! (RFC 3339). A local build keeps a fixed epoch so `cargo build` does not
//! rebuild the crate on every invocation.

fn main() {
    println!("cargo:rerun-if-env-changed=NAVIDOG_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=NAVIDOG_BUILD_TIMESTAMP");
    let channel = std::env::var("NAVIDOG_BUILD_CHANNEL").unwrap_or_else(|_| "stable".into());
    let timestamp =
        std::env::var("NAVIDOG_BUILD_TIMESTAMP").unwrap_or_else(|_| "1970-01-01T00:00:00Z".into());
    println!("cargo:rustc-env=NAVIDOG_BUILD_CHANNEL={channel}");
    println!("cargo:rustc-env=NAVIDOG_BUILD_TIMESTAMP={timestamp}");

    // Host cfg: the Windows icon resource is compiled on a Windows runner.
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icons/navidog.ico");
        res.compile().expect("compile the Windows icon resource");
    }
}
