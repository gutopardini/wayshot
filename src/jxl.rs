use std::ffi::{c_int, c_void};
use std::path::Path;
use std::ptr;

use gdk_pixbuf::Pixbuf;
use libloading::{Library, Symbol};

const JXL_ENC_SUCCESS: c_int = 0;
const JXL_ENC_NEED_MORE_OUTPUT: c_int = 2;
const JXL_TYPE_UINT8: c_int = 2;
const JXL_NATIVE_ENDIAN: c_int = 0;

type Encoder = c_void;
type FrameSettings = c_void;
type CreateEncoder = unsafe extern "C" fn(*const c_void) -> *mut Encoder;
type DestroyEncoder = unsafe extern "C" fn(*mut Encoder);
type InitBasicInfo = unsafe extern "C" fn(*mut BasicInfo);
type SetBasicInfo = unsafe extern "C" fn(*mut Encoder, *const BasicInfo) -> c_int;
type SetToSrgb = unsafe extern "C" fn(*mut ColorEncoding, c_int);
type SetColorEncoding = unsafe extern "C" fn(*mut Encoder, *const ColorEncoding) -> c_int;
type CreateFrameSettings =
    unsafe extern "C" fn(*mut Encoder, *const FrameSettings) -> *mut FrameSettings;
type SetFrameLossless = unsafe extern "C" fn(*mut FrameSettings, c_int) -> c_int;
type AddImageFrame =
    unsafe extern "C" fn(*const FrameSettings, *const PixelFormat, *const c_void, usize) -> c_int;
type CloseInput = unsafe extern "C" fn(*mut Encoder);
type ProcessOutput = unsafe extern "C" fn(*mut Encoder, *mut *mut u8, *mut usize) -> c_int;

#[repr(C)]
struct PreviewHeader {
    xsize: u32,
    ysize: u32,
}

#[repr(C)]
struct AnimationHeader {
    tps_numerator: u32,
    tps_denominator: u32,
    num_loops: u32,
    have_timecodes: c_int,
}

// Keep this layout synchronized with libjxl's public JxlBasicInfo structure.
// It is used only with the dynamically loaded C API, so libjxl remains an
// optional runtime capability instead of a link-time dependency.
#[repr(C)]
struct BasicInfo {
    have_container: c_int,
    xsize: u32,
    ysize: u32,
    bits_per_sample: u32,
    exponent_bits_per_sample: u32,
    intensity_target: f32,
    min_nits: f32,
    relative_to_max_display: c_int,
    linear_below: f32,
    uses_original_profile: c_int,
    have_preview: c_int,
    have_animation: c_int,
    orientation: c_int,
    num_color_channels: u32,
    num_extra_channels: u32,
    alpha_bits: u32,
    alpha_exponent_bits: u32,
    alpha_premultiplied: c_int,
    preview: PreviewHeader,
    animation: AnimationHeader,
    intrinsic_xsize: u32,
    intrinsic_ysize: u32,
    padding: [u8; 100],
}

#[repr(C)]
struct ColorEncoding {
    color_space: c_int,
    white_point: c_int,
    white_point_xy: [f64; 2],
    primaries: c_int,
    primaries_red_xy: [f64; 2],
    primaries_green_xy: [f64; 2],
    primaries_blue_xy: [f64; 2],
    transfer_function: c_int,
    gamma: f64,
    rendering_intent: c_int,
}

#[repr(C)]
struct PixelFormat {
    num_channels: u32,
    data_type: c_int,
    endianness: c_int,
    align: usize,
}

struct EncoderGuard {
    encoder: *mut Encoder,
    destroy: DestroyEncoder,
}

impl Drop for EncoderGuard {
    fn drop(&mut self) {
        if !self.encoder.is_null() {
            unsafe { (self.destroy)(self.encoder) };
        }
    }
}

fn load_library() -> Result<Library, String> {
    let mut errors = Vec::new();
    for name in ["libjxl.so.0.11", "libjxl.so.0.10", "libjxl.so"] {
        // SAFETY: Loading a known system library is safe as long as symbols
        // are checked before use. The returned Library owns the handle.
        match unsafe { Library::new(name) } {
            Ok(library) => return Ok(library),
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }
    Err(format!("libjxl is unavailable ({})", errors.join(", ")))
}

fn load_symbol<'a, T>(library: &'a Library, name: &[u8]) -> Result<Symbol<'a, T>, String> {
    // SAFETY: The caller supplies the symbol's C ABI type and the Library is
    // kept alive for the returned Symbol's lifetime.
    unsafe {
        library
            .get(name)
            .map_err(|error| format!("{}: {error}", String::from_utf8_lossy(name)))
    }
}

pub fn can_save() -> bool {
    encode_pixels(1, 1, 3, &[0, 0, 0]).is_ok()
}

pub fn save(pixbuf: &Pixbuf, path: &Path) -> Result<(), String> {
    let width = pixbuf.width();
    let height = pixbuf.height();
    let channels = pixbuf.n_channels();
    if width <= 0 || height <= 0 {
        return Err("Image has no pixels".to_string());
    }
    if channels != 3 && channels != 4 {
        return Err(format!("Unsupported pixel layout with {channels} channels"));
    }

    let width_usize = width as usize;
    let height_usize = height as usize;
    let channels_usize = channels as usize;
    let row_bytes = width_usize
        .checked_mul(channels_usize)
        .ok_or_else(|| "Image row is too wide".to_string())?;
    let rowstride = pixbuf.rowstride() as usize;
    if rowstride < row_bytes {
        return Err("Invalid image row stride".to_string());
    }

    let source = pixbuf.read_pixel_bytes();
    let source = source.as_ref();
    let required_bytes = rowstride
        .checked_mul(height_usize)
        .ok_or_else(|| "Image buffer is too large".to_string())?;
    if source.len() < required_bytes {
        return Err("Image pixel buffer is incomplete".to_string());
    }

    let pixel_count = width_usize
        .checked_mul(height_usize)
        .and_then(|count| count.checked_mul(channels_usize))
        .ok_or_else(|| "Image is too large".to_string())?;
    let mut pixels = Vec::with_capacity(pixel_count);
    for row in source.chunks(rowstride).take(height_usize) {
        pixels.extend_from_slice(&row[..row_bytes]);
    }

    let encoded = encode_pixels(width as u32, height as u32, channels as u32, &pixels)?;
    std::fs::write(path, encoded).map_err(|error| error.to_string())
}

fn encode_pixels(width: u32, height: u32, channels: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    if channels != 3 && channels != 4 {
        return Err(format!("Unsupported pixel layout with {channels} channels"));
    }

    let expected_size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|count| count.checked_mul(channels as usize))
        .ok_or_else(|| "Image is too large".to_string())?;
    if pixels.len() != expected_size {
        return Err("Pixel buffer size does not match image dimensions".to_string());
    }

    let library = load_library()?;
    let create: CreateEncoder = *load_symbol(&library, b"JxlEncoderCreate\0")?;
    let destroy: DestroyEncoder = *load_symbol(&library, b"JxlEncoderDestroy\0")?;
    let init_basic_info: InitBasicInfo = *load_symbol(&library, b"JxlEncoderInitBasicInfo\0")?;
    let set_basic_info: SetBasicInfo = *load_symbol(&library, b"JxlEncoderSetBasicInfo\0")?;
    let set_to_srgb: SetToSrgb = *load_symbol(&library, b"JxlColorEncodingSetToSRGB\0")?;
    let set_color_encoding: SetColorEncoding =
        *load_symbol(&library, b"JxlEncoderSetColorEncoding\0")?;
    let create_frame_settings: CreateFrameSettings =
        *load_symbol(&library, b"JxlEncoderFrameSettingsCreate\0")?;
    let set_frame_lossless: SetFrameLossless =
        *load_symbol(&library, b"JxlEncoderSetFrameLossless\0")?;
    let add_image_frame: AddImageFrame = *load_symbol(&library, b"JxlEncoderAddImageFrame\0")?;
    let close_input: CloseInput = *load_symbol(&library, b"JxlEncoderCloseInput\0")?;
    let process_output: ProcessOutput = *load_symbol(&library, b"JxlEncoderProcessOutput\0")?;

    let encoder = unsafe { create(ptr::null()) };
    if encoder.is_null() {
        return Err("libjxl could not create an encoder".to_string());
    }
    let _encoder_guard = EncoderGuard { encoder, destroy };

    let mut basic_info: BasicInfo = unsafe { std::mem::zeroed() };
    unsafe { init_basic_info(&mut basic_info) };
    basic_info.xsize = width;
    basic_info.ysize = height;
    basic_info.num_color_channels = 3;
    basic_info.num_extra_channels = u32::from(channels == 4);
    basic_info.alpha_bits = if channels == 4 { 8 } else { 0 };
    basic_info.alpha_exponent_bits = 0;
    basic_info.alpha_premultiplied = 0;
    if unsafe { set_basic_info(encoder, &basic_info) } != JXL_ENC_SUCCESS {
        return Err("libjxl rejected the image metadata".to_string());
    }

    let mut color_encoding: ColorEncoding = unsafe { std::mem::zeroed() };
    unsafe { set_to_srgb(&mut color_encoding, 0) };
    if unsafe { set_color_encoding(encoder, &color_encoding) } != JXL_ENC_SUCCESS {
        return Err("libjxl rejected the sRGB color profile".to_string());
    }

    let settings = unsafe { create_frame_settings(encoder, ptr::null()) };
    if settings.is_null() {
        return Err("libjxl could not create frame settings".to_string());
    }
    if unsafe { set_frame_lossless(settings, 1) } != JXL_ENC_SUCCESS {
        return Err("libjxl could not enable lossless encoding".to_string());
    }

    let pixel_format = PixelFormat {
        num_channels: channels,
        data_type: JXL_TYPE_UINT8,
        endianness: JXL_NATIVE_ENDIAN,
        align: 0,
    };
    if unsafe {
        add_image_frame(
            settings,
            &pixel_format,
            pixels.as_ptr() as *const c_void,
            pixels.len(),
        )
    } != JXL_ENC_SUCCESS
    {
        return Err("libjxl rejected the image pixels".to_string());
    }
    unsafe { close_input(encoder) };

    let mut encoded = Vec::new();
    loop {
        let mut output_buffer = [0_u8; 64 * 1024];
        let mut next_out = output_buffer.as_mut_ptr();
        let mut available = output_buffer.len();
        let status = unsafe { process_output(encoder, &mut next_out, &mut available) };
        let written = output_buffer.len() - available;
        encoded.extend_from_slice(&output_buffer[..written]);
        match status {
            JXL_ENC_SUCCESS => break,
            JXL_ENC_NEED_MORE_OUTPUT => {}
            _ => return Err(format!("libjxl failed while encoding (status {status})")),
        }
    }

    if encoded.is_empty() {
        return Err("libjxl returned an empty image".to_string());
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::{can_save, encode_pixels, save};
    use gdk_pixbuf::Pixbuf;

    #[test]
    fn jxl_encoder_is_optional_but_valid_when_present() {
        if can_save() {
            assert!(can_save());
            let encoded = encode_pixels(1, 1, 4, &[0, 0, 0, 255]).expect("RGBA JXL");
            assert!(!encoded.is_empty());

            let pixbuf =
                Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 1, 1).expect("test pixbuf");
            pixbuf.fill(0xff0000ff);
            let path =
                std::env::temp_dir().join(format!("wayshot-jxl-test-{}.jxl", std::process::id()));
            save(&pixbuf, &path).expect("save JXL");
            assert!(std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 0));
            let _ = std::fs::remove_file(path);
        }
    }
}
