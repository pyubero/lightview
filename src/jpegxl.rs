//! JPEG XL, with the pure-Rust `jxl` crate. It took 0.8 s on a 12 MP lossy file against
//! jxl-oxide's 2 s. Its `image` adapter has no sample limit and stops at the first frame.
use crate::i18n::tr;
use crate::load::{Frame, MAX_ANIMATION_BYTES, MAX_CANVAS_BYTES, frame_delay};
use jxl::api::{Event, ExtraChannel, JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer, JxlPixelFormat};
use std::time::Duration;

/// Each sample takes up to 4 bytes while decoding, so this holds a file to about 1 GiB.
const MAX_SAMPLES: usize = 1 << 28;

pub fn is_jxl(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(b"\0\0\0\x0CJXL \r\n\x87\n")
}

/// Decodes every frame that fits the animation budget, always to 8-bit RGB(A).
pub fn decode(mut bytes: &[u8]) -> Result<Vec<Frame>, String> {
    let mut options = JxlDecoderOptions::default();
    options.sample_limit = Some(MAX_SAMPLES);
    let mut decoder = JxlDecoder::new(options);
    let e = |e: jxl::error::Error| e.to_string();
    if decoder.process(&mut bytes, None, None).map_err(e)? != Event::BasicInfo {
        return Err(tr("this file is damaged or cut short").into());
    }
    let info = decoder.basic_info().ok_or(tr("this file is damaged or cut short"))?.clone();
    let (w, h) = (u32::try_from(info.size.0).map_err(|e| e.to_string())?, u32::try_from(info.size.1).map_err(|e| e.to_string())?);
    if w as u64 * h as u64 * 4 > MAX_CANVAS_BYTES {
        return Err(tr("this image is too large to open").into());
    }
    let alpha = info.extra_channels.iter().any(|c| c.ec_type == ExtraChannel::Alpha);
    let (color_type, channels) = if alpha { (JxlColorType::Rgba, 4) } else { (JxlColorType::Rgb, 3) };
    decoder
        .set_pixel_format(JxlPixelFormat { color_type, color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }), extra_channel_format: vec![None; info.extra_channels.len()] })
        .map_err(e)?;
    let animated = info.animation.is_some();
    let row = w as usize * channels;
    let mut frames: Vec<Frame> = Vec::new();
    loop {
        let mut pixels = vec![0; row * h as usize];
        let mut delay = Duration::ZERO;
        let more = loop {
            let mut output = JxlOutputBuffer::new(&mut pixels, h as usize, row);
            match decoder.process(&mut bytes, Some(std::slice::from_mut(&mut output)), None) {
                Ok(Event::FrameHeader) => {
                    if animated {
                        delay = frame_delay(decoder.frame_header().and_then(|f| f.duration).unwrap_or(0.0) as u32);
                    }
                }
                Ok(Event::FrameComplete { has_more_frames }) => break has_more_frames,
                // A long or damaged animation plays as far as it got.
                _ if !frames.is_empty() => return Ok(frames),
                Ok(_) => return Err(tr("this file is damaged or cut short").into()),
                Err(error) => return Err(error.to_string()),
            }
        };
        frames.push(Frame { w, h, pixels, alpha, delay });
        if !more || (frames.len() + 1) * row * h as usize > MAX_ANIMATION_BYTES {
            return Ok(frames);
        }
    }
}
