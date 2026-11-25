fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico"); // first icon resource = what Explorer shows for the exe
        res.compile().expect("embedding the exe icon (needs rc.exe from the Windows SDK)");
    }
}
