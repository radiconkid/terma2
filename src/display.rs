//! Display logic
//!
//! Provides functions for determining how pages should be displayed:
//! - Whether to show a single page or spread
//! - How many pages to advance
//! - How to calculate the previous page index
//! - How to calculate progress-based page index

use std::path::PathBuf;

use crate::image;

/// Determine if the current page should be displayed as a single page.
///
/// Returns true if:
/// - force_single is true
/// - cover_mode is true and current_idx is 0 (cover page)
/// - current_idx is the last image
/// - The current image is landscape (width > height)
/// - The next image in a spread would be landscape
pub fn should_display_single(
    images: &[PathBuf],
    current_idx: usize,
    cover_mode: bool,
    force_single: bool,
) -> bool {
    if force_single {
        return true;
    }
    if cover_mode && current_idx == 0 {
        return false;
    }
    if current_idx + 1 >= images.len() {
        return true;
    }
    // Landscape images (width > height) should be displayed as single page
    if image::is_landscape(&images[current_idx]) {
        return true;
    }
    // If the next image in a spread would be landscape, display current as single too
    if current_idx + 1 < images.len() && image::is_landscape(&images[current_idx + 1]) {
        return true;
    }
    false
}

/// Get the number of pages to advance for the "next" action.
///
/// Returns 1 for single-page display, 2 for spread display.
pub fn get_display_step(
    images: &[PathBuf],
    current_idx: usize,
    cover_mode: bool,
    force_single: bool,
) -> usize {
    if force_single {
        return 1;
    }
    if cover_mode && current_idx == 0 {
        return 1;
    }
    // Landscape images are displayed as single page, so advance by 1
    if image::is_landscape(&images[current_idx]) {
        return 1;
    }
    // If the next image is landscape, advance by 1
    if current_idx + 1 < images.len() && image::is_landscape(&images[current_idx + 1]) {
        return 1;
    }
    2
}

/// Get the index of the previous page, accounting for spread/single display.
pub fn get_previous_page_index(
    images: &[PathBuf],
    current_idx: usize,
    cover_mode: bool,
    force_single: bool,
) -> usize {
    if cover_mode && current_idx <= 1 {
        return 0;
    }
    let start = if cover_mode { 1 } else { 0 };
    let mut slides = vec![0usize];
    let mut idx = start;
    while idx < current_idx {
        slides.push(idx);
        let step = get_display_step(images, idx, cover_mode, force_single);
        idx += step;
    }
    *slides.last().unwrap_or(&0)
}

/// Get the image index corresponding to a percentage (0-100) of progress.
pub fn get_progress_index(total_images: usize, percent: usize) -> usize {
    if total_images <= 1 {
        return 0;
    }
    let target = ((total_images as f64 * (percent as f64 / 100.0)) - 0.5).round() as usize;
    target.min(total_images.saturating_sub(1))
}

