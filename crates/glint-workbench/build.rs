fn main() {
    println!("cargo:rerun-if-changed=../glint-api/assets/glint-api-icon.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("../glint-api/assets/glint-api-icon.ico")
        .set("ProductName", "Glint Workbench")
        .set("FileDescription", "Glint Workbench")
        .set("OriginalFilename", "glint-workbench.exe")
        .compile()
        .expect("failed to compile Windows application resources");
}
