fn main() {
    println!("cargo:rerun-if-changed=assets/tror.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/tror.ico");
        if let Err(e) = resource.compile() {
            println!("cargo:warning=значок exe не добавлен: {e}");
        }
    }
}
