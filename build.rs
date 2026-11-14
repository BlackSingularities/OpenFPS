//! Windows executable metadata and transparent application icon.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=OPENFPS_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ver = std::env::var("OPENFPS_VERSION").unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("FileDescription", "OpenFPS")
        .set("ProductName", "OpenFPS")
        .set("CompanyName", "Adam Rędzikowski")
        .set("LegalCopyright", "Adam Rędzikowski · redzikowski.com")
        .set("ProductVersion", &ver)
        .set("FileVersion", &ver);
    if let Err(e) = res.compile() {
        println!("cargo:warning=ikona .exe pominięta: {e}");
    }
}
