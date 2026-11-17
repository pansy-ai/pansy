//! On Windows, embeds the icon: resource 1 is what NSH's window, the
//! taskbar and the Start Menu shortcut show.
fn main() {
    println!("cargo:rerun-if-changed=../packaging/pansy.ico");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../packaging/pansy.ico")
            .set("ProductName", "Pansy")
            .set("FileDescription", "Pansy");
        resource.compile().expect("compile the Windows resources");
    }
}
