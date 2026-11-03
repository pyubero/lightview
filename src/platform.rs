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

/// `path` as a `file://` URI, escaped so that a comma can't split `dbus-send`'s array.
#[cfg(target_os = "linux")]
fn file_uri(path: &Path) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
    use std::os::unix::ffi::OsStrExt;
    const ESCAPED: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'_').remove(b'.').remove(b'~');
    format!("file://{}", percent_encode(path.as_os_str().as_bytes(), ESCAPED))
}

/// Opens the file manager with `path` highlighted, or just its folder if the desktop can't do that.
#[cfg(target_os = "linux")]
pub fn show_in_folder(path: &Path) {
    let (uri, dir) = (file_uri(path), path.parent().unwrap_or(Path::new("/")).to_path_buf());
    std::thread::spawn(move || {
        let call = ["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"];
        let shown = command("dbus-send").args(call).arg(format!("array:string:{uri}")).arg("string:").output();
        if !shown.is_ok_and(|o| o.status.success()) {
            open(dir);
        }
    });
}

/// Opens Explorer with `path` highlighted. It takes `/select,` and the quoted path as one argument,
/// which `Command::arg`'s own quoting would break, and doesn't understand `canonicalize`'s `\\\\?\\` form.
#[cfg(windows)]
pub fn show_in_folder(path: &Path) {
    use std::os::windows::process::CommandExt;
    let path = path.to_string_lossy();
    let path = match path.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(&path).to_string(),
    };
    drop(command("explorer").raw_arg(format!("/select,\"{path}\"")).spawn());
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

/// The desktop's dark/light preference from the XDG desktop portal, which winit 0.30 doesn't read on
/// Linux. Takes ~35 ms, so call it off the UI thread.
#[cfg(target_os = "linux")]
pub fn system_theme() -> Option<eframe::egui::Theme> {
    let out = command("gdbus").args(["call", "--session", "--dest", "org.freedesktop.portal.Desktop", "--object-path", "/org/freedesktop/portal/desktop",
        "--method", "org.freedesktop.portal.Settings.ReadOne", "org.freedesktop.appearance", "color-scheme"]).output().ok()?;
    parse_color_scheme(&String::from_utf8_lossy(&out.stdout))
}

/// gdbus prints `(<uint32 1>,)`: 1 prefers dark, 2 prefers light, 0 has no preference.
#[cfg(target_os = "linux")]
fn parse_color_scheme(reply: &str) -> Option<eframe::egui::Theme> {
    match reply.trim().trim_start_matches("(<").trim_start_matches('<').split(['>', ',']).next()?.trim() {
        "uint32 1" => Some(eframe::egui::Theme::Dark),
        "uint32 2" => Some(eframe::egui::Theme::Light),
        _ => None,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn file_uris_are_escaped() {
        assert_eq!(file_uri(Path::new("/a b/c,d/café-1_2.jpg")), "file:///a%20b/c%2Cd/caf%C3%A9-1_2.jpg");
    }

    #[test]
    fn color_scheme_replies() {
        use eframe::egui::Theme;
        assert_eq!(parse_color_scheme("(<uint32 1>,)\n"), Some(Theme::Dark));
        assert_eq!(parse_color_scheme("(<<uint32 2>>,)"), Some(Theme::Light));
        assert_eq!(parse_color_scheme("(<uint32 0>,)"), None);
        assert_eq!(parse_color_scheme(""), None);
    }
}
