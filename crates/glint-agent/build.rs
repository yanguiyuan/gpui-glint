fn main() {
    println!("cargo:rerun-if-changed=assets/glint-agent-icon.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("assets/glint-agent-icon.ico")
        .set("ProductName", "Glint Agent")
        .set("FileDescription", "Glint Agent")
        .set("OriginalFilename", "glint-agent.exe")
        .compile()
        .expect("failed to compile Windows application resources");
}
