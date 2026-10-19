fn main() {
    println!("cargo:rerun-if-changed=assets/mars_icon.ico");
    println!("cargo:rerun-if-changed=assets/mars_logo.png");
    println!("cargo:rerun-if-changed=assets/Orbitron-Bold.ttf");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/mars_icon.ico");
        res.set("ProductName", "Deimos Reader");
        res.set(
            "FileDescription",
            "Deimos Reader - Ultra-Lightweight PDF Reader",
        );
        res.set("CompanyName", "SOL12.NET");
        res.set("LegalCopyright", "Copyright (c) SOL12.NET (MIT License)");
        let _ = res.compile();
    }
}
