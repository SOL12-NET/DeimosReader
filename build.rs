fn main() {
    println!("cargo:rerun-if-changed=assets/mars_icon.ico");
    println!("cargo:rerun-if-changed=assets/mars_logo.png");
    println!("cargo:rerun-if-changed=assets/Orbitron-Bold.ttf");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/mars_icon.ico");
        res.set("ProductName", "Deimos Reader");
        res.set("ProductVersion", "0.6 Beta");
        res.set("FileVersion", "0.6.0.0");
        res.set(
            "FileDescription",
            "Deimos Reader - Ultra-Lightweight PDF Reader (v0.6 Beta)",
        );
        res.set("CompanyName", "SOL12.NET");
        res.set("LegalCopyright", "Copyright (c) 2026 SOL12.NET (MIT License)");
        let _ = res.compile();
    }
}
