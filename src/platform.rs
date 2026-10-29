//! The few things that differ between Linux and Windows.
use std::ffi::OsStr;
use std::process::Command;

/// A command for a helper program (curl, tar…). On Windows it gets no console window, which
/// would otherwise flash up from a windowed app.
pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Opens a URL or a folder with the desktop's default app.
pub fn open(target: impl AsRef<OsStr>) {
    let program = if cfg!(windows) { "explorer" } else { "xdg-open" };
    drop(command(program).arg(target).spawn());
}
