//! libjpeg-turbo, loaded at run time when it's installed. Every failure returns `None`, and the
//! caller falls back to the pure-Rust decoder.
use image::RgbImage;
use std::ffi::{c_int, c_ulong, c_void};
use std::sync::OnceLock;

type Handle = *mut c_void;

struct Lib {
    init: unsafe extern "C" fn() -> Handle,
    header: unsafe extern "C" fn(Handle, *const u8, c_ulong, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> c_int,
    decompress: unsafe extern "C" fn(Handle, *const u8, c_ulong, *mut u8, c_int, c_int, c_int, c_int, c_int) -> c_int,
    destroy: unsafe extern "C" fn(Handle) -> c_int,
    // Keeps the function pointers above valid.
    _lib: libloading::Library,
}

const NAME: &str = if cfg!(windows) { "turbojpeg.dll" } else { "libturbojpeg.so.0" };
const TJPF_RGB: c_int = 0;
// The `image` crate's default allocation limit; bigger files go to it and its own checks.
const MAX_BYTES: u64 = 512 << 20;

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(|| {
        // SAFETY: loading runs libjpeg-turbo's initialisers, which have no side effects. The
        // symbol types match the TurboJPEG 2.x API (turbojpeg.h), which 3.x still exports.
        unsafe {
            let lib = libloading::Library::new(NAME).ok()?;
            Some(Lib {
                init: *lib.get(b"tjInitDecompress\0").ok()?,
                header: *lib.get(b"tjDecompressHeader3\0").ok()?,
                decompress: *lib.get(b"tjDecompress2\0").ok()?,
                destroy: *lib.get(b"tjDestroy\0").ok()?,
                _lib: lib,
            })
        }
    })
    .as_ref()
}

pub fn available() -> bool {
    lib().is_some()
}

/// Decodes a JPEG to RGB at 1/`den` size (`den` is 1, 2, 4 or 8). The bytes are untrusted.
pub fn decode(bytes: &[u8], den: u32) -> Option<RgbImage> {
    let lib = lib()?;
    let len = c_ulong::try_from(bytes.len()).ok()?;
    // SAFETY: the handle is null-checked, used only here and always destroyed. `bytes` and `dst`
    // outlive the calls. libjpeg-turbo picks the largest scale whose output fits in w × h, and
    // w × h is exactly the 1/den output (TJSCALED rounds up too), so it writes at most w × h × 3
    // bytes into `dst` (pitch 0 means w × 3).
    unsafe {
        let handle = (lib.init)();
        if handle.is_null() {
            return None;
        }
        let out = (|| {
            let (mut w, mut h, mut subsampling, mut colorspace) = (0, 0, 0, 0);
            if (lib.header)(handle, bytes.as_ptr(), len, &mut w, &mut h, &mut subsampling, &mut colorspace) != 0 || w <= 0 || h <= 0 {
                return None;
            }
            let (w, h) = ((w as u32).div_ceil(den), (h as u32).div_ceil(den));
            if w as u64 * h as u64 * 3 > MAX_BYTES {
                return None;
            }
            let mut dst = vec![0u8; w as usize * h as usize * 3];
            if (lib.decompress)(handle, bytes.as_ptr(), len, dst.as_mut_ptr(), w as c_int, 0, h as c_int, TJPF_RGB, 0) != 0 {
                return None;
            }
            RgbImage::from_raw(w, h, dst)
        })();
        (lib.destroy)(handle);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_full_and_scaled() {
        if !available() {
            eprintln!("libjpeg-turbo not installed; skipped");
            return;
        }
        let mut jpeg = Vec::new();
        RgbImage::from_pixel(64, 32, image::Rgb([200, 30, 30])).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        assert_eq!(decode(&jpeg, 1).unwrap().dimensions(), (64, 32));
        let half = decode(&jpeg, 2).unwrap();
        assert_eq!(half.dimensions(), (32, 16));
        assert!(half.get_pixel(16, 8)[0] > 150);
        assert!(decode(b"not a jpeg", 1).is_none());
    }
}
