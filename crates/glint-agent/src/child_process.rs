//! Agent-owned background processes must not allocate Windows console windows.
use std::{ffi::OsStr, process::Command};

fn hide_console(_command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _command.creation_flags(0x08000000); // CREATE_NO_WINDOW; keep redirected stdio.
    }
}

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    hide_console(&mut command);
    command
}

pub fn async_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    hide_console(command.as_std_mut());
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    const SCRIPT: &str = r#"Add-Type -Name ConsoleProbe -Namespace GlintTest -MemberDefinition '[System.Runtime.InteropServices.DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();'; [Console]::Write([GlintTest.ConsoleProbe]::GetConsoleWindow().ToInt64()); [Console]::Error.Write('captured stderr'); exit 7"#;
    #[test]
    fn synchronous_child_has_no_console_and_preserves_output_and_exit_status() {
        let output = command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "0");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "captured stderr");
    }
    #[tokio::test]
    async fn asynchronous_child_has_no_console_and_preserves_output_and_exit_status() {
        let output = async_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .output()
            .await
            .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "0");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "captured stderr");
    }
}
