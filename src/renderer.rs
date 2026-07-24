//! Image rendering
//!
//! Provides the `SixelRenderer` which handles image display via:
//! - chafa (Sixel/Kitty format conversion)
//! - Kitty icat (native protocol)
//! - WezTerm imgcat (native protocol)

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ::image::GenericImageView;

use crate::image;
use crate::terminal;

/// The image renderer, automatically selecting the best available method.
pub struct SixelRenderer {
    /// Whether chafa is available and usable
    pub use_chafa: bool,
    /// Whether the terminal is Kitty
    pub is_kitty: bool,
    /// Whether the terminal is WezTerm
    pub is_wezterm: bool,
    /// Whether chafa is required (not Kitty/WezTerm)
    #[allow(dead_code)]
    pub chafa_required: bool,
}

impl SixelRenderer {
    /// Create a new renderer, auto-detecting terminal capabilities.
    pub fn new() -> Self {
        let term_type = terminal::detect_terminal_type();
        let use_chafa = terminal::is_chafa_usable();
        let is_kitty = term_type == terminal::TerminalType::Kitty;
        let is_wezterm = term_type == terminal::TerminalType::WezTerm;
        let chafa_required = !is_kitty && !is_wezterm;

        let (is_wezterm, chafa_required) = if chafa_required && !use_chafa {
            log::debug!("SixelRenderer: chafa not available, probing with imgcat...");
            if probe_wezterm_imgcat() {
                log::debug!("SixelRenderer: imgcat probe succeeded, detected WezTerm!");
                (true, false)
            } else {
                (false, true)
            }
        } else {
            (is_wezterm, chafa_required)
        };

        if chafa_required && !use_chafa {
            log::debug!("SixelRenderer: chafa is required but not available!");
        }

        log::debug!(
            "SixelRenderer: chafa={}, kitty={}, wezterm={}, chafa_required={}",
            use_chafa,
            is_kitty,
            is_wezterm,
            chafa_required
        );

        Self {
            use_chafa,
            is_kitty,
            is_wezterm,
            chafa_required,
        }
    }

    /// Clear the screen.
    pub fn clear(&self) {
        let mut stdout = std::io::stdout();
        let _ = write!(stdout, "\x1b[2J\x1b[H");
        let _ = stdout.flush();
    }

    /// Convert an image to Sixel/Kitty data using chafa.
    fn sixel_convert(&self, image_path: &Path, cols: usize, rows: usize) -> Option<Vec<u8>> {
        if !self.use_chafa {
            return None;
        }

        let fmt = if self.is_kitty { "kitty" } else { "sixels" };

        let result = Command::new("chafa")
            .args([
                "-f",
                fmt,
                "--size",
                &format!("{}x{}", cols, rows),
                "--symbols",
                "all",
                "--optimize",
                "0",
                &image_path.to_string_lossy(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match result {
            Ok(output) => {
                if output.status.success() && !output.stdout.is_empty() {
                    Some(output.stdout)
                } else {
                    log::debug!(
                        "chafa failed: rc={}, stderr={:?}",
                        output.status,
                        String::from_utf8_lossy(
                            &output.stderr[..200.min(output.stderr.len())]
                        )
                    );
                    None
                }
            }
            Err(e) => {
                log::debug!("chafa error: {e}");
                None
            }
        }
    }

    /// Fallback display using Kitty icat or WezTerm imgcat.
    fn fallback_display(&self, image_path: &Path, display_cols: usize, img_height: usize) -> bool {
        if self.is_kitty {
            let result = Command::new("kitty")
                .args([
                    "+kitten",
                    "icat",
                    "--place",
                    &format!("{}x{}@0x0", display_cols, img_height),
                    &image_path.to_string_lossy(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if let Ok(status) = result {
                if status.success() {
                    log::debug!("Fallback: used Kitty icat");
                    return true;
                }
            }
            log::debug!("Kitty icat fallback error");
        }

        if self.is_wezterm {
            let result = Command::new("wezterm")
                .args(["imgcat", &image_path.to_string_lossy()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if let Ok(status) = result {
                if status.success() {
                    log::debug!("Fallback: used WezTerm imgcat");
                    return true;
                }
            }
            log::debug!("WezTerm imgcat fallback error");
        }

        false
    }

    /// Show a warning when no display method is available.
    fn show_no_display_warning(&self) {
        let mut stdout = std::io::stdout();
        let _ = write!(stdout, "\x1b[2J\x1b[H");
        let _ = stdout.flush();

        let term = std::env::var("TERM").unwrap_or_default();
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();

        let lines = vec![
            "⚠️  No image display method available!".to_string(),
            String::new(),
            "TerMa requires one of the following to display images:".to_string(),
            String::new(),
            "  Option 1: Install chafa (recommended)".to_string(),
            "    $ pacman -S chafa  # or apt install chafa / brew install chafa".to_string(),
            String::new(),
            "  Option 2: Use a compatible terminal".to_string(),
            "    - Kitty (uses built-in icat)".to_string(),
            "    - WezTerm (uses built-in imgcat)".to_string(),
            "    - foot, mlterm, or xterm with sixel support".to_string(),
            String::new(),
            format!("  Current terminal: TERM={term}, TERM_PROGRAM={term_program}"),
            String::new(),
            "Press any key to exit...".to_string(),
        ];

        let (h, w) = terminal::get_terminal_size();
        for (i, line) in lines.iter().enumerate() {
            let y = h.saturating_sub(lines.len()) / 2 + i + 1;
            if y < h.saturating_sub(1) {
                let _ = write!(
                    stdout,
                    "\x1b[{};1H{:<width$}",
                    y,
                    line,
                    width = w.saturating_sub(1)
                );
            }
        }
        let _ = stdout.flush();
    }

    /// Output sixel/kitty data to the terminal.
    fn output_sixel(&self, data: &[u8]) {
        let mut stdout = std::io::stdout();
        if self.is_kitty {
            let modified = inject_kitty_z_index(data, -1);
            let _ = stdout.write_all(&modified);
        } else {
            let _ = stdout.write_all(data);
        }
        let _ = write!(stdout, "\x1b[?25l");
        let _ = stdout.flush();
    }

    /// Center the cursor for image display.
    fn center_cursor(&self, display_cols: usize, term_width: usize) {
        if display_cols < term_width {
            let offset = (term_width - display_cols) / 2;
            if offset > 0 {
                let mut stdout = std::io::stdout();
                let _ = write!(stdout, "\x1b[1;{}H", offset + 1);
                let _ = stdout.flush();
            }
        }
    }

    /// Display a single image (cover or single page).
    pub fn display_single(&self, image_path: &Path, term_width: usize, term_height: usize) {
        let mut stdout = std::io::stdout();
        let _ = write!(stdout, "\x1b[H");
        let _ = stdout.flush();

        let max_h = std::cmp::max(1, term_height.saturating_sub(2));
        let aspect = image::get_image_aspect(image_path);
        let cell_ratio = image::get_cell_aspect_ratio();
        let mut display_cols =
            std::cmp::max(1, (max_h as f64 * aspect * cell_ratio) as usize);
        let mut img_height = max_h;

        if display_cols > term_width.saturating_sub(2) {
            let scale = (term_width.saturating_sub(2)) as f64 / display_cols as f64;
            display_cols = term_width.saturating_sub(2);
            img_height = std::cmp::max(1, (img_height as f64 * scale) as usize);
        }

        if self.use_chafa {
            if let Some(data) = self.sixel_convert(image_path, display_cols, img_height) {
                self.center_cursor(display_cols, term_width);
                self.output_sixel(&data);
                return;
            }
            log::debug!("chafa conversion failed, trying native fallback");
        }

        if self.is_kitty || self.is_wezterm {
            if !self.fallback_display(image_path, display_cols, img_height) {
                log::debug!("Native display failed for Kitty/WezTerm");
                self.show_no_display_warning();
            }
            return;
        }

        if let Some(data) = self.sixel_convert(image_path, display_cols, img_height) {
            self.center_cursor(display_cols, term_width);
            self.output_sixel(&data);
        } else {
            log::debug!("Sixel conversion failed, trying fallback display");
            if !self.fallback_display(image_path, display_cols, img_height) {
                log::debug!("Fallback display also failed");
                self.show_no_display_warning();
            }
        }
    }

    /// Display a cover page (same as single).
    pub fn display_cover(&self, image_path: &Path, term_width: usize, term_height: usize) {
        self.display_single(image_path, term_width, term_height);
    }

    /// Display a spread (two pages side by side).
    pub fn display_spread(
        &self,
        img_right: &Path,
        img_left: Option<&Path>,
        term_width: usize,
        term_height: usize,
    ) {
        let mut stdout = std::io::stdout();
        let _ = write!(stdout, "\x1b[H");
        let _ = stdout.flush();

        let max_h = std::cmp::max(1, term_height.saturating_sub(2));
        let cell_ratio = image::get_cell_aspect_ratio();

        if self.use_chafa && img_left.is_some() {
            if let Some(combined_data) =
                self.combine_and_convert(img_right, img_left.unwrap(), max_h, term_width, cell_ratio)
            {
                self.output_sixel(&combined_data);
                return;
            }
            log::debug!("Sixel spread conversion failed, trying fallback display");
        }

        if (self.is_kitty || self.is_wezterm) && img_left.is_some() {
            if let Some(tmp_path) = self.combine_images_temp(img_right, img_left.unwrap()) {
                let aspect_r = image::get_image_aspect(img_right);
                let aspect_l = image::get_image_aspect(img_left.unwrap());
                let combined_aspect = aspect_l + aspect_r;
                let mut display_cols = std::cmp::max(
                    1,
                    (max_h as f64 * combined_aspect * cell_ratio) as usize,
                );
                let mut img_height = max_h;
                if display_cols > term_width.saturating_sub(2) {
                    let scale = (term_width.saturating_sub(2)) as f64 / display_cols as f64;
                    display_cols = term_width.saturating_sub(2);
                    img_height = std::cmp::max(1, (img_height as f64 * scale) as usize);
                }
                if self.fallback_display(&tmp_path, display_cols, img_height) {
                    let _ = std::fs::remove_file(&tmp_path);
                    return;
                }
                let _ = std::fs::remove_file(&tmp_path);
                log::debug!("Native spread display failed");
            }
        }

        if self.is_kitty || self.is_wezterm {
            self.display_single(img_right, term_width, term_height);
            return;
        }

        if img_left.is_some() {
            if let Some(combined_data) =
                self.combine_and_convert(img_right, img_left.unwrap(), max_h, term_width, cell_ratio)
            {
                self.output_sixel(&combined_data);
                return;
            }
            log::debug!("Sixel spread conversion failed, trying fallback display");
        }

        self.display_single(img_right, term_width, term_height);
    }

    /// Combine two images and convert using chafa.
    fn combine_and_convert(
        &self,
        img_right: &Path,
        img_left: &Path,
        max_h: usize,
        term_width: usize,
        cell_ratio: f64,
    ) -> Option<Vec<u8>> {
        let im_l = crate::image::open_image(img_left).ok()?;
        let im_r = crate::image::open_image(img_right).ok()?;

        let (w_l, h_l) = im_l.dimensions();
        let (w_r, h_r) = im_r.dimensions();

        let target_h = std::cmp::max(h_l, h_r);
        let new_w_l = (w_l as f64 * target_h as f64 / h_l as f64) as u32;
        let new_w_r = (w_r as f64 * target_h as f64 / h_r as f64) as u32;

        let im_l_resized = ::image::imageops::resize(
            &im_l,
            new_w_l,
            target_h,
            ::image::imageops::FilterType::Lanczos3,
        );
        let im_r_resized = ::image::imageops::resize(
            &im_r,
            new_w_r,
            target_h,
            ::image::imageops::FilterType::Lanczos3,
        );

        let combined_w = new_w_l + new_w_r;
        let mut combined = ::image::RgbImage::new(combined_w, target_h);

        for y in 0..target_h {
            for x in 0..new_w_l {
                let p = im_l_resized.get_pixel(x, y);
                combined.put_pixel(x, y, ::image::Rgb([p[0], p[1], p[2]]));
            }
        }
        for y in 0..target_h {
            for x in 0..new_w_r {
                let p = im_r_resized.get_pixel(x, y);
                combined.put_pixel(new_w_l + x, y, ::image::Rgb([p[0], p[1], p[2]]));
            }
        }

        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!("terma_spread_{}.png", std::process::id()));
        let _ = combined.save(&tmp_path);

        let aspect = combined_w as f64 / target_h as f64;
        let mut display_cols =
            std::cmp::max(1, (max_h as f64 * aspect * cell_ratio) as usize);
        let mut img_height = max_h;
        if display_cols > term_width.saturating_sub(2) {
            let scale = (term_width.saturating_sub(2)) as f64 / display_cols as f64;
            display_cols = term_width.saturating_sub(2);
            img_height = std::cmp::max(1, (img_height as f64 * scale) as usize);
        }

        let result = self.sixel_convert(&tmp_path, display_cols, img_height);
        let _ = std::fs::remove_file(&tmp_path);
        result
    }

    /// Combine two images into a temp file and return the path.
    fn combine_images_temp(&self, img_right: &Path, img_left: &Path) -> Option<PathBuf> {
        let im_l = crate::image::open_image(img_left).ok()?;
        let im_r = crate::image::open_image(img_right).ok()?;

        let (w_l, h_l) = im_l.dimensions();
        let (w_r, h_r) = im_r.dimensions();

        let target_h = std::cmp::max(h_l, h_r);
        let new_w_l = (w_l as f64 * target_h as f64 / h_l as f64) as u32;
        let new_w_r = (w_r as f64 * target_h as f64 / h_r as f64) as u32;

        let im_l_resized = ::image::imageops::resize(
            &im_l,
            new_w_l,
            target_h,
            ::image::imageops::FilterType::Lanczos3,
        );
        let im_r_resized = ::image::imageops::resize(
            &im_r,
            new_w_r,
            target_h,
            ::image::imageops::FilterType::Lanczos3,
        );

        let combined_w = new_w_l + new_w_r;
        let mut combined = ::image::RgbImage::new(combined_w, target_h);

        for y in 0..target_h {
            for x in 0..new_w_l {
                let p = im_l_resized.get_pixel(x, y);
                combined.put_pixel(x, y, ::image::Rgb([p[0], p[1], p[2]]));
            }
        }
        for y in 0..target_h {
            for x in 0..new_w_r {
                let p = im_r_resized.get_pixel(x, y);
                combined.put_pixel(new_w_l + x, y, ::image::Rgb([p[0], p[1], p[2]]));
            }
        }

        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!("terma_spread_{}.png", std::process::id()));
        let _ = combined.save(&tmp_path);
        Some(tmp_path)
    }
}

/// Probe the terminal by writing a small test image via the WezTerm/iTerm2
/// imgcat escape sequence directly to stdout.
fn probe_wezterm_imgcat() -> bool {
    // Create a tiny 1x1 PNG in memory
    fn png_chunk(chunk_type: &[u8], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        let len = data.len() as u32;
        chunk.extend_from_slice(&len.to_be_bytes());
        chunk.extend_from_slice(chunk_type);
        chunk.extend_from_slice(data);

        // CRC32
        let crc = {
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(chunk_type);
            hasher.update(data);
            hasher.finalize()
        };
        chunk.extend_from_slice(&crc.to_be_bytes());
        chunk
    }

    let mut png_data = Vec::new();
    // PNG signature
    png_data.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    // IHDR: 1x1 pixel, 8-bit RGB
    let ihdr_data = {
        let mut d = Vec::new();
        d.extend_from_slice(&1u32.to_be_bytes()); // width
        d.extend_from_slice(&1u32.to_be_bytes()); // height
        d.push(8); // bit depth
        d.push(2); // color type (RGB)
        d.push(0); // compression
        d.push(0); // filter
        d.push(0); // interlace
        d
    };
    png_data.extend_from_slice(&png_chunk(b"IHDR", &ihdr_data));
    // IDAT: raw pixel data (1 red pixel)
    let raw = b"\x00\xff\x00\x00"; // filter byte + RGB
    let compressed = {
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = encoder.write_all(raw);
        encoder.finish().unwrap_or_default()
    };
    png_data.extend_from_slice(&png_chunk(b"IDAT", &compressed));
    // IEND
    png_data.extend_from_slice(&png_chunk(b"IEND", b""));

    // Base64 encode
    let b64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);

    // Write the imgcat escape sequence directly to stdout
    let size = png_data.len();
    let mut stdout = std::io::stdout();
    let _ = write!(
        stdout,
        "\x1b]1337;File=inline=1;size={size}:{b64_data}\x07",
    );
    let _ = stdout.flush();
    log::debug!("_probe_wezterm_imgcat: probe sent successfully");
    true
}

/// Inject z-index parameter into Kitty graphics protocol sequences.
fn inject_kitty_z_index(data: &[u8], z: i32) -> Vec<u8> {
    // Find the first \x1b_G sequence
    let idx = match data.windows(2).position(|w| w == b"\x1b_G") {
        Some(i) => i,
        None => return data.to_vec(),
    };

    // Find parameter end (; or \x1b\\)
    let rest = &data[idx + 2..];
    let param_end = rest.iter().position(|&b| b == b';' || b == b'\x1b');

    let param_end = match param_end {
        Some(p) => p,
        None => return data.to_vec(),
    };

    let params_str = std::str::from_utf8(&rest[..param_end]).unwrap_or("");
    let new_params = if params_str.contains("z=") {
        // Replace existing z parameter
        let re = regex_lite::Regex::new(r"z=[^,]*").unwrap();
        re.replace(params_str, format!("z={z}")).to_string()
    } else {
        format!("{params_str},z={z}")
    };

    let mut result = Vec::with_capacity(data.len() + 16);
    result.extend_from_slice(&data[..idx + 2]);
    result.extend_from_slice(new_params.as_bytes());
    result.extend_from_slice(&rest[param_end..]);
    result
}

