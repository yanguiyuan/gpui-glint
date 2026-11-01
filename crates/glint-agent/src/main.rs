#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--glint-code-mode-worker") {
        glint_agent::run_code_mode_worker();
        return;
    }
    glint_agent::run();
}
