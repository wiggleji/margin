//! Windows: put Margin's icon and version details into margin.exe, so
//! Explorer, the Start menu and the taskbar show them. Nothing elsewhere.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/margin.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/windows/margin.ico");
    res.set("ProductName", "Margin");
    res.set("FileDescription", "Margin");
    res.set("CompanyName", "Margin contributors");
    res.set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        // a missing resource compiler shouldn't stop the build: only the icon is lost
        println!("cargo:warning=margin.exe built without its icon: {e}");
    }
}
