//! Image utility functions
//!
//! Provides functions for getting image aspect ratio, terminal cell pixel size,
//! and cell aspect ratio. Mirrors the functionality from terma.py.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Cache for image aspect ratios to avoid repeated decoding.
static ASPECT_CACHE: std::sync::LazyLock<Mutex<HashMap<String, f64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Get the aspect ratio (width/height) of an image.
///
/// Returns the aspect ratio, or a default of 0.7 if the image cannot be read.
/// Results are cached to avoid repeated decoding.
pub fn get_image_aspect(path: &Path) -> f64 {
    let path_str = path.to_string_lossy().to_string();

    // Check cache first
    {
        let cache = ASPECT_CACHE.lock().unwrap();
        if let Some(&cached) = cache.get(&path_str) {
            return cached;
        }
    }

    // Try to read just the dimensions without full decoding
    let aspect = match image::ImageReader::open(path) {
        Ok(reader) => match reader.into_dimensions() {
            Ok((w, h)) if w > 0 && h > 0 => f64::from(w) / f64::from(h),
            _ => 0.7,
        },
        Err(_) => 0.7,
    };

    // Store in cache
    {
        let mut cache = ASPECT_CACHE.lock().unwrap();
        cache.insert(path_str, aspect);
    }

    aspect
}

/// Get the terminal's cell pixel size (width, height) in physical pixels.
///
/// Uses TIOCGWINSZ ioctl on Unix/Linux to get the terminal window size
/// in pixels and characters, then computes the cell size.
/// Returns None if the information cannot be obtained.
#[cfg(unix)]
pub fn get_cell_pixel_size() -> Option<(f64, f64)> {
    use std::fs;
    use std::os::unix::io::AsRawFd;

    let stdout = fs::File::open("/dev/tty").ok()?;
    let fd = stdout.as_raw_fd();

    // SAFETY: ioctl is safe as long as we pass a valid buffer.
    unsafe {
        let mut winsize: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut winsize) == 0 {
            let cols = winsize.ws_col;
            let rows = winsize.ws_row;
            let xpixel = winsize.ws_xpixel;
            let ypixel = winsize.ws_ypixel;

            if cols > 0 && rows > 0 && xpixel > 0 && ypixel > 0 {
                let cell_w = f64::from(xpixel) / f64::from(cols);
                let cell_h = f64::from(ypixel) / f64::from(rows);
                return Some((cell_w, cell_h));
            }
        }
    }
    None
}

/// Get the terminal's cell pixel size (stub for non-Unix platforms).
#[cfg(not(unix))]
pub fn get_cell_pixel_size() -> Option<(f64, f64)> {
    None
}

/// Get the terminal's cell aspect ratio (height/width).
///
/// Returns the ratio, or a default of 2.45 if the cell size cannot be determined.
pub fn get_cell_aspect_ratio() -> f64 {
    if let Some((cell_w, cell_h)) = get_cell_pixel_size() {
        if cell_w > 0.0 {
            return cell_h / cell_w;
        }
    }
    2.45
}

/// Check if an image is landscape (width > height).
pub fn is_landscape(path: &Path) -> bool {
    get_image_aspect(path) > 1.0
}
