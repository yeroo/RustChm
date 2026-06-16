// Embeds Windows version/publisher metadata (VERSIONINFO) into the .exe so it
// presents as proper, identifiable software rather than an anonymous binary.
// Build-time only; runs on Windows targets, a no-op elsewhere.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        {
            let mut res = winresource::WindowsResource::new();
            res.set("ProductName", "rustchm");
            res.set("FileDescription", "rustchm — CHM (HTML Help) compiler and reader");
            res.set("CompanyName", "yeroo");
            res.set("LegalCopyright", "Copyright (c) 2026 yeroo. MIT License.");
            res.set("OriginalFilename", "rustchm.exe");
            if let Err(e) = res.compile() {
                println!("cargo:warning=version-info embed skipped: {e}");
            }
        }
    }
}
