fn main() {
    println!("cargo:rerun-if-changed=assets/glint-api-icon.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("assets/glint-api-icon.ico")
        .set("ProductName", "Glint API")
        .set("FileDescription", "Glint API")
        .set("OriginalFilename", "glint-api.exe")
        .compile()
        .expect("failed to compile Windows application resources");
}
