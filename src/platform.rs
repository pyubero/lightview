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

/// The user's locale, like `es-ES`.
// shortcut: this is the regional format, not the display language; use GetUserPreferredUILanguages if they often differ.
#[cfg(windows)]
pub fn locale() -> Option<String> {
    let mut name = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let len = unsafe { windows_sys::Win32::Globalization::GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (len > 0).then(|| String::from_utf16_lossy(&name[..len as usize - 1]))
}

/// Linux has no call for this: the language comes from the environment.
#[cfg(not(windows))]
pub fn locale() -> Option<String> {
    None
}
