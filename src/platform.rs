//! The few things that differ between Linux and Windows.
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// A command for a helper program (curl, tar, PowerShell…). On Windows it gets no console window, which
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

/// Downloads `url` to `dest` with the system's curl.
// shortcut: a helper program rather than an HTTP crate; fine while downloads are few and need no progress bar.
#[cfg(not(windows))]
pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    crate::ai::run(command("curl").args(["-L", "--fail", "--silent", "--show-error", "-o"]).arg(dest).arg(url))
}

/// Downloads `url` to `dest` with PowerShell: not every Windows has curl.exe, as seen on a Windows 11 PC.
#[cfg(windows)]
pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    run_powershell(
        "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        (New-Object Net.WebClient).DownloadFile($env:LV_URL, $env:LV_DEST)",
        &[("LV_URL", url.as_ref()), ("LV_DEST", dest.as_ref())],
    )
}

/// Unpacks the zip entries matching `patterns` (`*` wildcards) straight into `dest`, without their folders.
#[cfg(windows)]
pub fn unzip(archive: &Path, dest: &Path, patterns: &[&str]) -> Result<(), String> {
    run_powershell(
        "Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [IO.Compression.ZipFile]::OpenRead($env:LV_ARCHIVE)
        try {
            foreach ($entry in $zip.Entries) {
                if ($env:LV_PATTERNS -split '\\|' | Where-Object { $entry.Name -and $entry.FullName -like $_ }) {
                    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, [IO.Path]::Combine($env:LV_DEST, $entry.Name), $true)
                }
            }
        } finally { $zip.Dispose() }",
        &[("LV_ARCHIVE", archive.as_ref()), ("LV_DEST", dest.as_ref()), ("LV_PATTERNS", patterns.join("|").as_ref())],
    )
}

/// Runs a script, with its values in the environment so they need no quoting. A failure gives just its message.
#[cfg(windows)]
fn run_powershell(script: &str, env: &[(&str, &OsStr)]) -> Result<(), String> {
    let script = format!("$ErrorActionPreference = 'Stop'; try {{ {script} }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}");
    crate::ai::run(command("powershell").args(["-NoProfile", "-NonInteractive", "-Command"]).arg(script).envs(env.iter().copied()))
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
