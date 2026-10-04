fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=Failed to compile Windows icon resource: {}", e);
        }
    }
}
