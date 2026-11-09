use crate::i18n::{tr, trf};
use crate::load::{EXTENSIONS, Msg, Notify};
use crate::settings::SortOrder;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// What tells two versions of a file apart: when it was modified, and its length.
pub type Stamp = (SystemTime, u64);

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Checks the path put in the returned slot every `interval`, on a thread of its own so that it
/// takes no repaints and a slow network folder can't hold up the UI. A change is sent once it has
/// held for a whole interval, not to load a half-written file. A file that's gone sends nothing.
pub fn watch(notify: Notify, interval: Duration) -> Arc<Mutex<Option<PathBuf>>> {
    let slot = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&slot);
    std::thread::spawn(move || {
        let (mut path, mut known, mut last) = (None, None, None);
        while Arc::strong_count(&shared) > 1 {
            std::thread::sleep(interval);
            let now: Option<PathBuf> = shared.lock().ok().and_then(|p| p.clone());
            let current = now.as_deref().and_then(stamp);
            if now != path {
                (path, known, last) = (now, current, current);
                continue;
            }
            if let (Some(p), Some(new)) = (&path, current) {
                if known.is_none() {
                    known = current;
                } else if current != known && current == last {
                    known = current;
                    notify.send(Msg::Changed(p.clone(), new));
                }
            }
            last = current;
        }
    });
    slot
}

/// `fs::canonicalize` without Windows' `\\?\` prefix: it showed in Copy Path, Info and Open Recent,
/// and file dialogs and Explorer don't take it. Every path compared with an open image comes from here.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(simplify)
}

/// `path` without a `\\?\` prefix, kept when the path is too long to work without it.
pub fn simplify(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else { return path };
    let plain = match text.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => match text.strip_prefix(r"\\?\") {
            Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
            _ => return path,
        },
    };
    if plain.len() < 260 { PathBuf::from(plain) } else { path }
}

/// Whether a Trash item is `path`. When Windows hides known extensions, `item.name` has none,
/// but `item.id` (like `$RAB12CD.jpg`) keeps it.
fn is_trashed_copy(item: &trash::TrashItem, path: &Path) -> bool {
    let same_name = Some(item.name.as_os_str()) == path.file_name() || (Some(item.name.as_os_str()) == path.file_stem() && Path::new(&item.id).extension() == path.extension());
    // Compared canonically, as `path` is: the Trash may spell the folder differently.
    same_name && canonicalize(&item.original_parent).is_ok_and(|p| Some(p.as_path()) == path.parent())
}

/// Puts a file back from the Trash, to the place it was deleted from.
pub fn restore_trashed(path: &Path) -> Result<(), String> {
    let items = trash::os_limited::list().map_err(|e| e.to_string())?;
    let newest = items.into_iter().filter(|i| is_trashed_copy(i, path)).max_by_key(|i| i.time_deleted);
    let item = newest.ok_or_else(|| tr("It's no longer in the Trash").to_string())?;
    // The full name, or the file would come back without its extension.
    let name = path.file_name().unwrap_or_default().to_owned();
    trash::os_limited::restore_all([trash::TrashItem { name, ..item }]).map_err(|e| e.to_string())
}

/// Renames the file `path` to `name` in the same folder. Never replaces another file.
// shortcut: changing only the case on a case-insensitive file system counts as "already exists"; handle it if it's reported.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(tr("That isn't a valid file name").into());
    }
    let new = path.with_file_name(name);
    // `fs::rename` would silently replace it. This also sees a link to nowhere.
    if new.symlink_metadata().is_ok() {
        return Err(trf("{name} already exists", &[("name", &name)]));
    }
    std::fs::rename(path, &new).map_err(|e| e.to_string())?;
    Ok(new)
}

/// Whether `path` has the extension of a format lightview opens.
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The first image in folder `dir`, in `sort` order.
pub fn first_image(dir: &Path, sort: SortOrder) -> Option<PathBuf> {
    // `list_folder` lists a file's siblings, and gives the file back when it can't read the folder.
    let child = dir.join("-");
    list_folder(&child, sort).into_iter().find(|p| *p != child)
}

/// The supported images in `path`'s folder: by name in file-manager order, or oldest first.
pub fn list_folder(path: &Path, sort: SortOrder) -> Vec<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![path.to_path_buf()] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| is_image(p))
        .collect();
    let name = |p: &PathBuf| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let by_name = |a: &PathBuf, b: &PathBuf| {
        let (a, b) = (name(a), name(b));
        natural_cmp(&a, &b).then_with(|| a.cmp(&b))
    };
    match sort {
        SortOrder::Name => files.sort_by(by_name),
        SortOrder::DateModified => {
            let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let mut dated: Vec<_> = files.into_iter().map(|p| (modified(&p), p)).collect();
            dated.sort_by(|(ta, a), (tb, b)| ta.cmp(tb).then_with(|| by_name(a, b)));
            files = dated.into_iter().map(|(_, p)| p).collect();
        }
    }
    files
}

/// Case-insensitive comparison where runs of digits compare as numbers ("img2" < "img10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.next_if(|c| c.is_ascii_digit()) {
                        s.push(c);
                    }
                    s.trim_start_matches('0').to_string()
                };
                let (x, y) = (digits(&mut a), digits(&mut b));
                let ord = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn the_watcher_reports_a_settled_change_once() {
        let dir = std::env::temp_dir().join(format!("lightview-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        std::fs::write(&path, b"1").unwrap();
        let (tx, rx) = mpsc::channel();
        let every = Duration::from_millis(20);
        let slot = watch(Notify { tx, ctx: Arc::default() }, every);
        *slot.lock().unwrap() = Some(path.clone());
        let wait = Duration::from_millis(150);
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err(), "just watching isn't a change");
        std::fs::write(&path, b"22").unwrap();
        let Ok(Msg::Changed(changed, stamp_seen)) = rx.recv_timeout(Duration::from_secs(5)) else { panic!("no change seen") };
        assert_eq!((changed, Some(stamp_seen)), (path.clone(), stamp(&path)));
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err(), "one message per change");
        std::fs::remove_file(&path).unwrap();
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err(), "gone isn't a change");
        // Replaced, as an editor saving through a temporary file does.
        std::fs::write(&path, b"333").unwrap();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)), Ok(Msg::Changed(..))));
        // Another file starts clean, and so does none.
        let other = dir.join("b.png");
        std::fs::write(&other, b"4").unwrap();
        *slot.lock().unwrap() = Some(other);
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err());
        *slot.lock().unwrap() = None;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_never_replaces_a_file() {
        let dir = std::env::temp_dir().join(format!("lightview-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert!(rename(&a, "b.png").is_err());
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        for bad in ["", ".", "..", "../c.png", "sub/c.png", "sub\\c.png"] {
            assert!(rename(&a, bad).is_err(), "{bad}");
        }
        assert!(a.exists());
        assert_eq!(rename(&a, "c d.png").unwrap(), dir.join("c d.png"));
        assert!(!a.exists() && dir.join("c d.png").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_trashed_file_with_hidden_extension() {
        let dir = canonicalize(&std::env::temp_dir()).unwrap();
        let path = dir.join("photo.jpg");
        let item = |name: &str, id: &str| trash::TrashItem { id: id.into(), name: name.into(), original_parent: dir.clone(), time_deleted: 0 };
        assert!(is_trashed_copy(&item("photo.jpg", "/trash/info/photo.jpg.trashinfo"), &path));
        assert!(is_trashed_copy(&item("photo", r"C:\$Recycle.Bin\S-1\$RAB12CD.jpg"), &path));
        assert!(!is_trashed_copy(&item("photo", r"C:\$Recycle.Bin\S-1\$RAB12CD.png"), &path));
        assert!(!is_trashed_copy(&item("other.jpg", "/trash/info/other.jpg.trashinfo"), &path));
        assert!(!is_trashed_copy(&trash::TrashItem { original_parent: dir.join("elsewhere"), ..item("photo.jpg", "x") }, &path));
    }

    // Uses the real Trash: `cargo test trash_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn trash_round_trip() {
        let path = std::env::temp_dir().join(format!("lightview-trash-{}.txt", std::process::id()));
        std::fs::write(&path, "image").unwrap();
        let path = canonicalize(&path).unwrap();
        trash::delete(&path).unwrap();
        assert!(!path.exists());
        restore_trashed(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "image");
        std::fs::remove_file(&path).unwrap();
        assert!(restore_trashed(&path).is_err());
    }

    #[test]
    fn canonical_paths_have_no_verbatim_prefix() {
        assert_eq!(simplify(PathBuf::from(r"\\?\D:\Fotos\DSC ñ 1.jpg")), PathBuf::from(r"D:\Fotos\DSC ñ 1.jpg"));
        assert_eq!(simplify(PathBuf::from(r"\\?\UNC\server\share\a.jpg")), PathBuf::from(r"\\server\share\a.jpg"));
        let long = format!(r"\\?\D:\{}.jpg", "a".repeat(300));
        assert_eq!(simplify(PathBuf::from(&long)), PathBuf::from(&long), "too long without it");
        assert_eq!(simplify(PathBuf::from("/home/a.jpg")), PathBuf::from("/home/a.jpg"));
        let dir = canonicalize(&std::env::temp_dir()).unwrap();
        assert!(!dir.to_string_lossy().starts_with(r"\\?\"), "{dir:?}");
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("img2.jpg", "img10.jpg"), Ordering::Less);
        assert_eq!(natural_cmp("IMG_b.jpg", "img_A.jpg"), Ordering::Greater);
        assert_eq!(natural_cmp("a007", "a7"), Ordering::Equal);
        assert_eq!(natural_cmp("photo", "photo1"), Ordering::Less);
    }

    #[test]
    fn folder_filter_and_order() {
        let dir = std::env::temp_dir().join(format!("lightview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["b10.JPG", "b2.png", "a.webp", "notes.txt", "clip.mp4", "layers.psd"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        // Oldest: b2.png, then a.webp, then b10.JPG.
        let base = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for (f, secs) in [("b2.png", 0), ("a.webp", 10), ("b10.JPG", 20)] {
            std::fs::File::options().write(true).open(dir.join(f)).unwrap().set_modified(base + Duration::from_secs(secs)).unwrap();
        }
        let names = |sort| -> Vec<String> {
            list_folder(&dir.join("a.webp"), sort).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        let (by_name, by_date) = (names(SortOrder::Name), names(SortOrder::DateModified));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(by_name, ["a.webp", "b2.png", "b10.JPG"]);
        assert_eq!(by_date, ["b2.png", "a.webp", "b10.JPG"]);
    }
}
