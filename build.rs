//! Embeds the app icon and version information into Courier.exe on Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/courier.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/courier.ico");
        res.set("ProductName", "Courier");
        res.set("FileDescription", "Courier API client");
        res.set("LegalCopyright", "Copyright © 2026 Courier contributors");
        res.compile().expect("failed to embed Windows resources");
    }
}
