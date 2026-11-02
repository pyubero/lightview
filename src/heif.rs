//! HEIC and AVIF, decoded by the system: libheif on Linux, loaded at run time like libjpeg-turbo,
//! and Windows' own codecs (WIC) there. Both are native code, so they only ever get the image on
//! screen: `load` goes by extension and `App` leaves these files out of decoding ahead.
use crate::i18n::tr;
use crate::load::MAX_CANVAS_BYTES;
use image::{DynamicImage, RgbaImage};
use std::path::Path;

/// Files that can only be decoded by native code.
// shortcut: a real HEIC misnamed .jpg fails to open; sniff in `load` if that's reported.
pub fn is_native(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| crate::load::NATIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn fits(w: u64, h: u64) -> Result<(), String> {
    if w * h * 4 > MAX_CANVAS_BYTES { Err(tr("this image is too large to open").into()) } else { Ok(()) }
}

#[cfg(not(windows))]
pub use libheif::decode;
#[cfg(windows)]
pub use wic::decode;

#[cfg(not(windows))]
mod libheif {
    use super::*;
    use image::RgbImage;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::ptr::{null, null_mut};
    use std::sync::OnceLock;

    type Context = *mut c_void;
    type Handle = *mut c_void;
    type Image = *mut c_void;

    #[repr(C)]
    struct Error {
        code: c_int,
        subcode: c_int,
        message: *const c_char,
    }

    pub(super) struct Lib {
        context_alloc: unsafe extern "C" fn() -> Context,
        context_free: unsafe extern "C" fn(Context),
        read: unsafe extern "C" fn(Context, *const u8, usize, *const c_void) -> Error,
        primary: unsafe extern "C" fn(Context, *mut Handle) -> Error,
        width: unsafe extern "C" fn(Handle) -> c_int,
        height: unsafe extern "C" fn(Handle) -> c_int,
        has_alpha: unsafe extern "C" fn(Handle) -> c_int,
        decode: unsafe extern "C" fn(Handle, *mut Image, c_int, c_int, *const c_void) -> Error,
        handle_release: unsafe extern "C" fn(Handle),
        image_width: unsafe extern "C" fn(Image, c_int) -> c_int,
        image_height: unsafe extern "C" fn(Image, c_int) -> c_int,
        plane: unsafe extern "C" fn(Image, c_int, *mut c_int) -> *const u8,
        image_release: unsafe extern "C" fn(Image),
        // Keeps the function pointers above valid.
        _lib: libloading::Library,
    }

    const COLORSPACE_RGB: c_int = 1;
    const CHROMA_RGB: c_int = 10;
    const CHROMA_RGBA: c_int = 11;
    const CHANNEL_INTERLEAVED: c_int = 10;
    const NO_DECODER: [c_int; 2] = [3000, 6003];

    pub(super) fn lib() -> Option<&'static Lib> {
        static LIB: OnceLock<Option<Lib>> = OnceLock::new();
        LIB.get_or_init(|| {
            // SAFETY: loading runs libheif's initialisers, and the symbol types match its C API,
            // stable since 1.12 (Ubuntu 22.04, the oldest system lightview supports).
            unsafe {
                let lib = libloading::Library::new("libheif.so.1").ok()?;
                Some(Lib {
                    context_alloc: *lib.get(b"heif_context_alloc\0").ok()?,
                    context_free: *lib.get(b"heif_context_free\0").ok()?,
                    read: *lib.get(b"heif_context_read_from_memory_without_copy\0").ok()?,
                    primary: *lib.get(b"heif_context_get_primary_image_handle\0").ok()?,
                    width: *lib.get(b"heif_image_handle_get_width\0").ok()?,
                    height: *lib.get(b"heif_image_handle_get_height\0").ok()?,
                    has_alpha: *lib.get(b"heif_image_handle_has_alpha_channel\0").ok()?,
                    decode: *lib.get(b"heif_decode_image\0").ok()?,
                    handle_release: *lib.get(b"heif_image_handle_release\0").ok()?,
                    image_width: *lib.get(b"heif_image_get_width\0").ok()?,
                    image_height: *lib.get(b"heif_image_get_height\0").ok()?,
                    plane: *lib.get(b"heif_image_get_plane_readonly\0").ok()?,
                    image_release: *lib.get(b"heif_image_release\0").ok()?,
                    _lib: lib,
                })
            }
        })
        .as_ref()
    }

    /// `Ok` unless libheif reported an error. Call it right away: `message` belongs to libheif.
    unsafe fn check(error: Error) -> Result<(), String> {
        if error.code == 0 {
            Ok(())
        } else if NO_DECODER.contains(&error.subcode) {
            Err(tr("this file needs a libheif decoder plugin, which isn't installed").into())
        } else {
            // SAFETY: libheif documents `message` as always set.
            Err(unsafe { CStr::from_ptr(error.message) }.to_string_lossy().into_owned())
        }
    }

    /// The top-level image, upright (libheif applies the file's rotation). The bytes are untrusted.
    pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
        decode_with(lib(), bytes)
    }

    pub(super) fn decode_with(lib: Option<&Lib>, bytes: &[u8]) -> Result<DynamicImage, String> {
        let lib = lib.ok_or(tr("HEIC and AVIF files need libheif, which isn't installed"))?;
        // SAFETY: each pointer is null-checked or comes with a successful call, is used only here
        // and is always released. `bytes` outlives the context, which doesn't copy it.
        unsafe {
            let context = (lib.context_alloc)();
            if context.is_null() {
                return Err(tr("HEIC and AVIF files need libheif, which isn't installed").into());
            }
            let out = (|| {
                check((lib.read)(context, bytes.as_ptr(), bytes.len(), null()))?;
                let mut handle = null_mut();
                check((lib.primary)(context, &mut handle))?;
                let out = (|| {
                    fits((lib.width)(handle).max(0) as u64, (lib.height)(handle).max(0) as u64)?;
                    let alpha = (lib.has_alpha)(handle) != 0;
                    let mut image = null_mut();
                    check((lib.decode)(handle, &mut image, COLORSPACE_RGB, if alpha { CHROMA_RGBA } else { CHROMA_RGB }, null()))?;
                    let out = copy(lib, image, alpha);
                    (lib.image_release)(image);
                    out
                })();
                (lib.handle_release)(handle);
                out
            })();
            (lib.context_free)(context);
            out
        }
    }

    /// The decoded rows without their padding.
    unsafe fn copy(lib: &Lib, image: Image, alpha: bool) -> Result<DynamicImage, String> {
        // SAFETY: `image` is a live, successfully decoded image, whose plane holds `h` rows of `stride`
        // bytes, of which each row's first `w` × `channels` are checked to be inside the stride.
        unsafe {
            let (w, h) = ((lib.image_width)(image, CHANNEL_INTERLEAVED), (lib.image_height)(image, CHANNEL_INTERLEAVED));
            let mut stride = 0;
            let plane = (lib.plane)(image, CHANNEL_INTERLEAVED, &mut stride);
            let channels = if alpha { 4 } else { 3 };
            let (width, height) = (w.max(0) as usize, h.max(0) as usize);
            let row = width * channels;
            if plane.is_null() || w <= 0 || h <= 0 || (stride as usize) < row || stride < 0 {
                return Err(tr("this file is damaged or cut short").into());
            }
            fits(width as u64, height as u64)?;
            let mut pixels = Vec::with_capacity(row * height);
            for y in 0..height {
                pixels.extend_from_slice(std::slice::from_raw_parts(plane.add(y * stride as usize), row));
            }
            let img = if alpha { RgbaImage::from_raw(w as u32, h as u32, pixels).map(DynamicImage::ImageRgba8) } else { RgbImage::from_raw(w as u32, h as u32, pixels).map(DynamicImage::ImageRgb8) };
            img.ok_or_else(|| tr("this file is damaged or cut short").into())
        }
    }
}

#[cfg(windows)]
mod wic {
    use super::*;
    use windows::Win32::Foundation::WINCODEC_ERR_COMPONENTNOTFOUND;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;
    use windows::core::Interface;

    /// The top-level image through whichever codecs this PC has. The bytes are untrusted.
    pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
        // SAFETY: WIC's calls are used as documented; `bytes` and `pixels` outlive them, and the
        // buffer is exactly `w` × `h` × 4 bytes, which is what `CopyPixels` is told.
        let result = unsafe {
            // Once per thread: repeats just report that it's done, and a thread that's in another
            // mode still works, so the result doesn't matter.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            (|| -> windows::core::Result<Result<DynamicImage, String>> {
                let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let stream = factory.CreateStream()?;
                stream.InitializeFromMemory(bytes)?;
                let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
                let frame: IWICBitmapSource = decoder.GetFrame(0)?.cast()?;
                let rgba = WICConvertBitmapSource(&GUID_WICPixelFormat32bppRGBA, &frame)?;
                let (mut w, mut h) = (0, 0);
                rgba.GetSize(&mut w, &mut h)?;
                if let Err(e) = fits(w as u64, h as u64) {
                    return Ok(Err(e));
                }
                let mut pixels = vec![0; w as usize * h as usize * 4];
                rgba.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
                Ok(RgbaImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgba8).ok_or_else(|| tr("this file is damaged or cut short").into()))
            })()
        };
        result.unwrap_or_else(|e| {
            Err(if e.code() == WINCODEC_ERR_COMPONENTNOTFOUND { tr("this file needs Microsoft's HEIF Image Extensions and HEVC or AV1 Video Extensions").into() } else { e.message() })
        })
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::libheif::*;
    use super::*;

    #[test]
    fn a_missing_library_says_so() {
        assert_eq!(decode_with(None, b"").err().as_deref(), Some(tr("HEIC and AVIF files need libheif, which isn't installed")));
    }

    #[test]
    fn decodes_heic_and_avif_upright() {
        if lib().is_none() {
            eprintln!("libheif not installed; skipped");
            return;
        }
        for name in ["tiny.heic", "tiny.avif"] {
            let img = decode(&std::fs::read(format!("tests/data/{name}")).unwrap()).unwrap().to_rgb8();
            assert_eq!(img.dimensions(), (64, 48), "{name}");
            let ([r, _, b], [r2, _, b2]) = (img.get_pixel(8, 24).0, img.get_pixel(56, 24).0);
            assert!(r > 180 && b < 80 && r2 < 80 && b2 > 180, "{name}: {r} {b} {r2} {b2}");
        }
        assert!(decode(b"\0\0\0\x18ftypheic not really").is_err());
    }

    #[test]
    fn rotation_is_applied() {
        if lib().is_some() {
            let img = decode(&std::fs::read("tests/data/rotated.heic").unwrap()).unwrap();
            assert_eq!((img.width(), img.height()), (48, 64));
        }
    }

    #[test]
    fn cut_short_files_fail_cleanly() {
        if lib().is_some() {
            let bytes = std::fs::read("tests/data/tiny.heic").unwrap();
            for len in 0..bytes.len() {
                let _ = decode(&bytes[..len]);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    // Wine and a bare Windows have no HEIF codec, which must say what to install.
    #[test]
    fn decodes_or_names_the_missing_extension() {
        match decode(&std::fs::read("tests/data/tiny.heic").unwrap()) {
            Ok(img) => assert_eq!((img.width(), img.height()), (64, 48)),
            Err(e) => assert!(e == tr("this file needs Microsoft's HEIF Image Extensions and HEVC or AV1 Video Extensions"), "{e}"),
        }
        assert!(decode(b"not an image").is_err());
    }
}
