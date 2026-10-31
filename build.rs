fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icon/lightview.ico");
    // Windows takes the Start menu, taskbar and file icons from the .exe itself.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new().set_icon("icon/lightview.ico").compile().unwrap();
    }
}
