fn main() {
    println!("cargo:rerun-if-changed=icons/app.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");

    #[cfg(windows)]
    {
        // GPUI loads the application icon from Windows resource ID 1.
        let mut resources = winres::WindowsResource::new();
        resources
            .set_icon("icons/app.ico")
            .set("ProductName", "EdolWave")
            .set("FileDescription", "EdolWave audio player")
            .set("OriginalFilename", "edolwave.exe");
        resources
            .compile()
            .expect("Failed to embed Windows resources");
    }
}
