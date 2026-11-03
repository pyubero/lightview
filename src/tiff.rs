use std::collections::HashSet;

/// A TIFF's pages, including the thumbnails that are skipped.
pub const MAX_TIFF_IFDS: usize = 4096;

/// The JPEG thumbnail in IFD1 of an EXIF (TIFF) block, if any. The file is untrusted:
/// every read is bounds-checked and anything malformed returns `None`.
pub fn exif_thumbnail(tiff: &[u8]) -> Option<&[u8]> {
    tiff.get(thumbnail_range(tiff)?)
}

/// Where `exif_thumbnail` is, if that's inside the block.
pub fn thumbnail_range(tiff: &[u8]) -> Option<std::ops::Range<usize>> {
    let (u16_at, u32_at) = tiff_readers(tiff)?;
    let ifd1 = u32_at(next_ifd_ptr(tiff)?)?;
    if ifd1 == 0 {
        return None;
    }
    let (mut offset, mut len) = (None, None);
    for i in 0..u16_at(ifd1)? {
        let entry = ifd1 + 2 + i * 12;
        match u16_at(entry)? {
            0x0201 => offset = u32_at(entry + 8),
            0x0202 => len = u32_at(entry + 8),
            _ => {}
        }
    }
    let offset = offset?;
    let range = offset..offset.checked_add(len?)?;
    tiff.get(range.clone()).map(|_| range)
}

pub type Reader<'a> = Box<dyn Fn(usize) -> Option<usize> + 'a>;

/// Bounds-checked u16 and u32 readers in the TIFF block's byte order.
/// Readers for a TIFF structure's integers. Olympus (`IIRO`, `MMOR`) and Panasonic (`IIU\0`)
/// RAW files change only the magic number.
pub fn tiff_readers(tiff: &[u8]) -> Option<(Reader<'_>, Reader<'_>)> {
    let le = match tiff.get(0..4)? {
        [b'I', b'I', 42, 0] | [b'I', b'I', b'R', b'O'] | [b'I', b'I', b'U', 0] => true,
        [b'M', b'M', 0, 42] | [b'M', b'M', b'O', b'R'] => false,
        _ => return None,
    };
    let u16_at = move |o: usize| {
        let b = tiff.get(o..o.checked_add(2)?)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) } as usize)
    };
    let u32_at = move |o: usize| {
        let b: [u8; 4] = tiff.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) } as usize)
    };
    Some((Box::new(u16_at), Box::new(u32_at)))
}

/// Offset of IFD0's "next IFD" pointer, which points at IFD1 (the thumbnail), if it's in bounds.
pub fn next_ifd_ptr(tiff: &[u8]) -> Option<usize> {
    let (u16_at, u32_at) = tiff_readers(tiff)?;
    let ifd0 = u32_at(4)?;
    let ptr = ifd0 + 2 + u16_at(ifd0)? * 12;
    u32_at(ptr).map(|_| ptr)
}

/// Whether the chain of IFDs in this TIFF block ends: a parser that follows every link never
/// stops on one that loops. Whatever can't be read counts as the end.
pub fn ifd_chain_ends(tiff: &[u8]) -> bool {
    let Some((u16_at, u32_at)) = tiff_readers(tiff) else { return true };
    // A multi-page TIFF has an IFD per page, so this goes by repeats, not length.
    let (mut ifd, mut seen) = (u32_at(4), HashSet::new());
    while seen.len() < MAX_TIFF_IFDS {
        match ifd {
            None | Some(0) => return true,
            Some(at) if !seen.insert(at) => return false,
            Some(at) => ifd = u16_at(at).and_then(|entries| u32_at(at + 2 + entries * 12)),
        }
    }
    false
}

/// Erases the EXIF thumbnail, for when the pixels no longer match it (e.g. after rotating), and
/// unlinks it. Only unlinking would leave the old picture in the file, cropped-out parts included.
// shortcut: a preview in the MakerNote survives; clear that too if Save As should drop all of them.
pub fn drop_exif_thumbnail(tiff: &mut [u8]) {
    let ptr = next_ifd_ptr(tiff);
    if let Some(range) = thumbnail_range(tiff) {
        tiff[range].fill(0);
    }
    if let Some(ptr) = ptr {
        tiff[ptr..ptr + 4].fill(0);
    }
}

/// Sets IFD0's XResolution and YResolution to `dpi` and ResolutionUnit to inches, so the
/// copied EXIF agrees with the DPI written into the saved file. Tags that aren't there are
/// left out (the file's own header still has the DPI); malformed input is left alone.
pub fn set_exif_dpi(tiff: &mut [u8], dpi: u16) {
    let mut writes: Vec<(usize, [u32; 2], bool)> = Vec::new();
    {
        let Some((u16_at, u32_at)) = tiff_readers(tiff) else { return };
        let Some(ifd0) = u32_at(4) else { return };
        for i in 0..u16_at(ifd0).unwrap_or(0) {
            let entry = ifd0 + 2 + i * 12;
            match (u16_at(entry), u16_at(entry + 2)) {
                // RATIONAL: the value is 8 bytes elsewhere.
                (Some(0x011A | 0x011B), Some(5)) => {
                    if let Some(at) = u32_at(entry + 8).filter(|&at| u32_at(at + 4).is_some()) {
                        writes.push((at, [dpi as u32, 1], true));
                    }
                }
                // SHORT: the value sits in the entry.
                (Some(0x0128), Some(3)) if u16_at(entry + 8).is_some() => writes.push((entry + 8, [2, 0], false)),
                (Some(_), Some(_)) => {}
                _ => return,
            }
        }
    }
    let le = tiff[0] == b'I';
    for (at, [a, b], rational) in writes {
        if rational {
            tiff[at..at + 4].copy_from_slice(&if le { a.to_le_bytes() } else { a.to_be_bytes() });
            tiff[at + 4..at + 8].copy_from_slice(&if le { b.to_le_bytes() } else { b.to_be_bytes() });
        } else {
            tiff[at..at + 2].copy_from_slice(&if le { (a as u16).to_le_bytes() } else { (a as u16).to_be_bytes() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TIFF block with an empty IFD0 and an IFD1 pointing at `thumb`.
    fn tiff(le: bool, thumb: &[u8]) -> Vec<u8> {
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut t = Vec::new();
        t.extend(if le { *b"II\x2a\0" } else { *b"MM\0\x2a" });
        t.extend(u32b(8)); // IFD0 at 8
        t.extend(u16b(0)); // IFD0: no entries
        t.extend(u32b(14)); // IFD1 at 14
        t.extend(u16b(2)); // IFD1: two entries
        let thumb_at = 14 + 2 + 2 * 12 + 4;
        for (tag, value) in [(0x0201, thumb_at), (0x0202, thumb.len() as u32)] {
            t.extend(u16b(tag));
            t.extend(u16b(4)); // LONG
            t.extend(u32b(1));
            t.extend(u32b(value));
        }
        t.extend(u32b(0)); // no IFD2
        t.extend(thumb);
        t
    }

    #[test]
    fn exif_thumbnail_found() {
        assert_eq!(exif_thumbnail(&tiff(true, b"JPEGDATA")), Some(&b"JPEGDATA"[..]));
        assert_eq!(exif_thumbnail(&tiff(false, b"JPEGDATA")), Some(&b"JPEGDATA"[..]));
    }

    #[test]
    fn drop_thumbnail() {
        for le in [true, false] {
            let mut t = tiff(le, b"JPEGDATA");
            drop_exif_thumbnail(&mut t);
            assert_eq!(exif_thumbnail(&t), None);
        }
        // Malformed input is left alone without panicking.
        let mut junk = b"II\x2a\0\xff\xff\xff\xff".to_vec();
        drop_exif_thumbnail(&mut junk);
        assert_eq!(junk, b"II\x2a\0\xff\xff\xff\xff");
    }

    #[test]
    fn exif_dpi_malformed() {
        // IFD0 with XResolution (RATIONAL at 26) and ResolutionUnit, then 72/1.
        let mut good = b"II\x2a\0\x08\0\0\0\x02\0".to_vec();
        good.extend([0x1A, 0x01, 5, 0, 1, 0, 0, 0, 34, 0, 0, 0]);
        good.extend([0x28, 0x01, 3, 0, 1, 0, 0, 0, 3, 0, 0, 0]);
        good.extend([0, 0, 0, 0, 72, 0, 0, 0, 1, 0, 0, 0]);
        let mut t = good.clone();
        set_exif_dpi(&mut t, 300);
        assert_eq!(&t[34..42], &[44, 1, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&t[30..32], &[2, 0]);
        // Truncated anywhere: no panic.
        for len in 0..good.len() {
            set_exif_dpi(&mut good[..len].to_vec(), 300);
        }
    }

    #[test]
    fn exif_thumbnail_malformed() {
        let good = tiff(true, b"JPEGDATA");
        // Every truncation of a valid block fails cleanly.
        for len in 0..good.len() {
            assert_eq!(exif_thumbnail(&good[..len]), None);
        }
        assert_eq!(exif_thumbnail(b"garbage garbage garbage"), None);
        // Offsets pointing far out of bounds.
        let mut bad = good.clone();
        bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(exif_thumbnail(&bad), None);
    }

    #[test]
    fn dropped_thumbnail_leaves_no_trace() {
        for le in [true, false] {
            let mut t = tiff(le, b"JPEGDATA");
            drop_exif_thumbnail(&mut t);
            assert!(!t.windows(8).any(|w| w == b"JPEGDATA"), "the old picture is still in the file");
        }
    }

    #[test]
    fn ifd_chains() {
        let ifd = |at: usize, next: u32| {
            let mut t = b"II\x2a\0".to_vec();
            t.extend((at as u32).to_le_bytes());
            t.resize(at, 0);
            t.extend([0, 0]);
            t.extend(next.to_le_bytes());
            t
        };
        assert!(ifd_chain_ends(&ifd(8, 0)));
        assert!(ifd_chain_ends(&ifd(8, 0xFFFF_FFFF)), "a link out of bounds is the end");
        assert!(!ifd_chain_ends(&ifd(8, 8)), "links to itself");
        assert!(ifd_chain_ends(b"II"));
    }
}
