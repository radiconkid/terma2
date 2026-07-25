//! Safe wrapper around the chafa library (libchafa) for converting images
//! to terminal graphics formats (Sixel, Kitty, iTerm2).
//!
//! This replaces the previous approach of calling the `chafa` CLI binary
//! as an external process. It now uses the same rendering pipeline as the
//! CLI: `chafa_canvas_set_placement()` + `chafa_canvas_print_rows()`.

use std::path::Path;

/// Pixel mode for chafa output.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum PixelMode {
    /// ANSI art using character symbols
    Symbols,
    /// Sixel graphics format
    Sixels,
    /// Kitty terminal graphics protocol
    Kitty,
    /// iTerm2 terminal graphics protocol
    Iterm2,
}

/// A chafa canvas for converting images to terminal graphics.
///
/// Uses the full rendering pipeline (placement + print_rows) matching
/// the chafa CLI for best quality.
pub struct ChafaCanvas {
    symbol_map: *mut chafa_sys::ChafaSymbolMap,
    fill_symbol_map: *mut chafa_sys::ChafaSymbolMap,
    config: *mut chafa_sys::ChafaCanvasConfig,
    canvas: *mut chafa_sys::ChafaCanvas,
    term_info: *mut chafa_sys::ChafaTermInfo,
    term_db: *mut chafa_sys::ChafaTermDb,
}

// SAFETY: ChafaCanvas only contains raw pointers to chafa objects
// which are thread-safe in the chafa library.
unsafe impl Send for ChafaCanvas {}
unsafe impl Sync for ChafaCanvas {}

impl ChafaCanvas {
    /// Create a new chafa canvas with the given dimensions (in terminal cells)
    /// and pixel mode.
    ///
    /// `cell_width` and `cell_height` are the terminal's cell pixel dimensions.
    /// Setting these correctly ensures chafa uses the proper aspect ratio when
    /// rendering the image. If not known, pass (8, 16) as a reasonable default.
    ///
    /// Configures all quality settings to match the chafa CLI defaults:
    /// - Preprocessing enabled
    /// - Dither mode: diffusion (Floyd-Steinberg)
    /// - Color extractor: average
    /// - Color space: DIN99d (perceptual)
    /// - Canvas mode: truecolor
    /// - Work factor: 0.5 (mid-quality/speed tradeoff)
    /// - Optimizations: none (best quality)
    #[allow(dead_code)]
    pub fn new(width: u32, height: u32, cell_width: u32, cell_height: u32, pixel_mode: PixelMode) -> Self {
        let symbol_map = unsafe {
            let map = chafa_sys::chafa_symbol_map_new();
            chafa_sys::chafa_symbol_map_add_by_tags(
                map,
                chafa_sys::ChafaSymbolTags_CHAFA_SYMBOL_TAG_ALL,
            );
            map
        };

        let fill_symbol_map = unsafe {
            let map = chafa_sys::chafa_symbol_map_new();
            chafa_sys::chafa_symbol_map_add_by_tags(
                map,
                chafa_sys::ChafaSymbolTags_CHAFA_SYMBOL_TAG_ALL,
            );
            map
        };

        let term_db = unsafe {
            chafa_sys::chafa_term_db_new()
        };

        let term_info = unsafe {
            // Use the fallback term info which has all pixel modes and
            // capabilities enabled (Sixel, Kitty, iTerm2, etc.).
            // A blank chafa_term_info_new() has no capabilities set,
            // which would cause chafa_canvas_print() to output only
            // symbols without color information.
            let info = chafa_sys::chafa_term_db_get_fallback_info(term_db);
            // The fallback info is owned by the term_db, so we need to
            // copy it to own it independently.
            chafa_sys::chafa_term_info_copy(info)
        };

        let config = unsafe {
            let cfg = chafa_sys::chafa_canvas_config_new();
            chafa_sys::chafa_canvas_config_set_geometry(cfg, width as i32, height as i32);
            chafa_sys::chafa_canvas_config_set_symbol_map(cfg, symbol_map);
            chafa_sys::chafa_canvas_config_set_fill_symbol_map(cfg, fill_symbol_map);

            // Disable all optimizations for best quality (matches CLI --optimize 0)
            chafa_sys::chafa_canvas_config_set_optimizations(
                cfg,
                chafa_sys::ChafaOptimizations_CHAFA_OPTIMIZATION_NONE,
            );

            // Enable preprocessing (matches CLI default)
            chafa_sys::chafa_canvas_config_set_preprocessing_enabled(
                cfg,
                true.into(),
            );

            // Use error diffusion dithering for best quality (matches CLI default)
            chafa_sys::chafa_canvas_config_set_dither_mode(
                cfg,
                chafa_sys::ChafaDitherMode_CHAFA_DITHER_MODE_DIFFUSION,
            );

            // Use average color extractor (matches CLI default)
            chafa_sys::chafa_canvas_config_set_color_extractor(
                cfg,
                chafa_sys::ChafaColorExtractor_CHAFA_COLOR_EXTRACTOR_AVERAGE,
            );

            // Use DIN99d perceptual color space (matches CLI default)
            chafa_sys::chafa_canvas_config_set_color_space(
                cfg,
                chafa_sys::ChafaColorSpace_CHAFA_COLOR_SPACE_DIN99D,
            );

            // Use truecolor mode (matches CLI default for pixel modes)
            chafa_sys::chafa_canvas_config_set_canvas_mode(
                cfg,
                chafa_sys::ChafaCanvasMode_CHAFA_CANVAS_MODE_TRUECOLOR,
            );

            // Set the terminal's cell geometry so chafa uses the correct pixel
            // dimensions per cell. This prevents aspect ratio distortion when
            // the terminal's cell size differs from chafa's default (8x16).
            chafa_sys::chafa_canvas_config_set_cell_geometry(
                cfg,
                cell_width as i32,
                cell_height as i32,
            );

            // Work factor: CLI normalizes [1..9] to [0.0..1.0], default is 5 -> 0.5
            chafa_sys::chafa_canvas_config_set_work_factor(cfg, 0.5);

            // Set pixel mode based on desired output format
            let mode = match pixel_mode {
                PixelMode::Symbols => chafa_sys::ChafaPixelMode_CHAFA_PIXEL_MODE_SYMBOLS,
                PixelMode::Sixels => chafa_sys::ChafaPixelMode_CHAFA_PIXEL_MODE_SIXELS,
                PixelMode::Kitty => chafa_sys::ChafaPixelMode_CHAFA_PIXEL_MODE_KITTY,
                PixelMode::Iterm2 => chafa_sys::ChafaPixelMode_CHAFA_PIXEL_MODE_ITERM2,
            };
            chafa_sys::chafa_canvas_config_set_pixel_mode(cfg, mode);

            cfg
        };

        let canvas = unsafe {
            chafa_sys::chafa_canvas_new(config)
        };

        Self {
            symbol_map,
            fill_symbol_map,
            config,
            canvas,
            term_info,
            term_db,
        }
    }

    /// Render RGBA8 pixel data onto the canvas using the full placement pipeline
    /// (matching chafa CLI behavior) and return the ANSI escape sequence string.
    ///
    /// This uses `chafa_canvas_set_placement()` + `chafa_canvas_print()`
    /// instead of the lower-level `draw_all_pixels()` + `build_ansi()`, which
    /// gives significantly better image quality.
    ///
    /// NOTE: `chafa_canvas_print()` returns a GString containing the full ANSI
    /// output with color information (Sixel, Kitty, etc.), unlike
    /// `chafa_canvas_print_rows_strv()` which only returns raw text without
    /// color escape sequences.
    pub fn render(&self, pixels: &[u8], pix_width: u32, pix_height: u32) -> String {
        unsafe {
            // Create a frame borrowing the pixel data (no copy)
            let frame = chafa_sys::chafa_frame_new_borrow(
                pixels.as_ptr() as *const libc::c_void,
                chafa_sys::ChafaPixelType_CHAFA_PIXEL_RGBA8_UNASSOCIATED,
                pix_width as i32,
                pix_height as i32,
                (pix_width * 4) as i32,
            );

            // Create an image and attach the frame
            let image = chafa_sys::chafa_image_new();
            chafa_sys::chafa_image_set_frame(image, frame);

            // Create a placement with FIT tuck (preserve aspect ratio, no padding)
            let placement = chafa_sys::chafa_placement_new(image, -1);
            chafa_sys::chafa_placement_set_tuck(
                placement,
                chafa_sys::ChafaTuck_CHAFA_TUCK_FIT,
            );
            chafa_sys::chafa_placement_set_halign(
                placement,
                chafa_sys::ChafaAlign_CHAFA_ALIGN_CENTER,
            );
            chafa_sys::chafa_placement_set_valign(
                placement,
                chafa_sys::ChafaAlign_CHAFA_ALIGN_CENTER,
            );

            // Set the placement on the canvas
            chafa_sys::chafa_canvas_set_placement(self.canvas, placement);

            // Print the full ANSI output using chafa_canvas_print().
            // This returns a GString containing color escape sequences
            // (Sixel data, Kitty protocol, etc.), unlike print_rows_strv
            // which only returns raw text without color information.
            let gs = chafa_sys::chafa_canvas_print(
                self.canvas,
                self.term_info,
            );

            // Extract the string and free the GString.
            // g_string_free with free_segment=FALSE returns the internal
            // char* buffer without freeing it, so we can copy it to Rust.
            let result = if !gs.is_null() {
                let c_str = chafa_sys::g_string_free(gs, false.into());
                let s = if !c_str.is_null() {
                    std::ffi::CStr::from_ptr(c_str)
                        .to_string_lossy()
                        .to_string()
                } else {
                    String::new()
                };
                // Free the C string buffer that was returned by g_string_free
                if !c_str.is_null() {
                    libc::free(c_str as *mut libc::c_void);
                }
                s
            } else {
                String::new()
            };

            // Unref in reverse order of creation
            chafa_sys::chafa_placement_unref(placement);
            chafa_sys::chafa_image_unref(image);
            chafa_sys::chafa_frame_unref(frame);

            result
        }
    }

    /// Check if chafa library is available (always true if this module compiles).
    pub fn is_available() -> bool {
        true
    }
}

impl Drop for ChafaCanvas {
    fn drop(&mut self) {
        unsafe {
            chafa_sys::chafa_canvas_unref(self.canvas);
            chafa_sys::chafa_canvas_config_unref(self.config);
            chafa_sys::chafa_symbol_map_unref(self.symbol_map);
            chafa_sys::chafa_symbol_map_unref(self.fill_symbol_map);
            chafa_sys::chafa_term_info_unref(self.term_info);
            chafa_sys::chafa_term_db_unref(self.term_db);
        }
    }
}

/// Convert an image file to terminal graphics data using the chafa library.
///
/// This is a high-level convenience function that:
/// 1. Opens and decodes the image using the `image` crate
/// 2. Creates a chafa canvas with the exact given cell dimensions
/// 3. Converts the image to the specified pixel mode using the full
///    placement rendering pipeline (same as chafa CLI)
/// 4. Returns the raw ANSI escape sequence bytes and the actual canvas dimensions
///
/// NOTE: `term_cols` and `term_rows` should already be calculated considering
/// the cell aspect ratio (e.g., by `display_single()` in renderer.rs).
/// This function does NOT call `chafa_calc_canvas_geometry()` to avoid
/// double-applying the aspect ratio adjustment.
pub fn convert_image(
    image_path: &Path,
    term_cols: u32,
    term_rows: u32,
    cell_width: u32,
    cell_height: u32,
    pixel_mode: PixelMode,
) -> Option<(Vec<u8>, (u32, u32))> {
    let img = image::ImageReader::open(image_path).ok()?.decode().ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());

    // Use the exact cell dimensions passed in, without calling
    // chafa_calc_canvas_geometry(), since the caller already calculated
    // the correct dimensions considering cell aspect ratio.
    let canvas = ChafaCanvas::new(term_cols, term_rows, cell_width, cell_height, pixel_mode);
    let ansi = canvas.render(rgba.as_raw(), w, h);

    if ansi.is_empty() {
        None
    } else {
        Some((ansi.into_bytes(), (term_cols, term_rows)))
    }
}

