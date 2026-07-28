//! Main application module
//!
//! Contains the main application loop (`run`), state management,
//! input handling, and display logic.

use std::path::{Path, PathBuf};

use crate::debug_log;
use crate::display;
use crate::fileops;
use crate::renderer::SixelRenderer;
use crate::resume;
use crate::terminal;

/// Run the manga viewer application with the given target path.
pub fn run(target_path: PathBuf) -> anyhow::Result<()> {
    // Save the last opened folder for external tools (e.g. yazi plugin)
    if let Some(canonical) = target_path.canonicalize().ok() {
        resume::set_last_opened_folder(&canonical.to_string_lossy());
    }

    let resume_key = target_path
        .canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    let (initial_dir, is_archive, archive_name, temp_dir) = if target_path.is_file() {
        let archive_filename = target_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let temp_dir_obj = tempfile::TempDir::with_prefix("terma_")?;
        let extracted_path = temp_dir_obj.path().to_path_buf();
        if fileops::extract_archive(&target_path, &extracted_path) {
            fileops::extract_nested_archives(&extracted_path);
            (
                extracted_path,
                true,
                Some(archive_filename),
                Some(temp_dir_obj),
            )
        } else {
            anyhow::bail!(
                "Error: {} is not a directory or a supported archive file.",
                target_path.display()
            );
        }
    } else {
        (target_path, false, None, None)
    };

    // Initialize terminal
    terminal::init_terminal()?;

    let result = run_app(
        &initial_dir,
        is_archive,
        archive_name.as_deref(),
        resume_key.as_deref(),
    );

    // Restore terminal
    terminal::restore_terminal()?;

    // Drop temp dir (cleans up extracted files)
    drop(temp_dir);

    result
}

/// Internal application loop.
fn run_app(
    initial_dir: &Path,
    is_archive: bool,
    archive_name: Option<&str>,
    resume_key: Option<&str>,
) -> anyhow::Result<()> {
    let renderer = SixelRenderer::new();

    // Determine directories to browse
    let archive_resume_base;
    let dirs_to_browse: Vec<PathBuf>;

    if is_archive {
        let mut dir = initial_dir.to_path_buf();
        // Auto-descend if only one subdirectory
        loop {
            let items = match std::fs::read_dir(&dir) {
                Ok(entries) => entries.filter_map(|e| e.ok()).collect::<Vec<_>>(),
                Err(_) => break,
            };
            let subdirs: Vec<_> = items
                .iter()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            let files: Vec<_> = items
                .iter()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .collect();
            if subdirs.len() == 1 && files.is_empty() {
                dir = subdirs[0].path();
            } else {
                break;
            }
        }

        archive_resume_base = Some(dir.clone());
        let extensions = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".avif"];

        let mut dirs_with_images = Vec::new();
        let has_images = |d: &Path| -> bool {
            std::fs::read_dir(d)
                .map(|entries| {
                    entries.filter_map(|e| e.ok()).any(|e| {
                        e.file_type().map(|t| t.is_file()).unwrap_or(false)
                            && e.path()
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .map(|ext| extensions.contains(&ext.to_lowercase().as_str()))
                                .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        };

        if has_images(&dir) {
            dirs_with_images.push(dir.clone());
        }

        // Walk directory tree for image-containing directories
        let walk_iter = walkdir::WalkDir::new(&dir)
            .sort_by(|a, b| {
                let a_name = a.file_name().to_string_lossy().to_string();
                let b_name = b.file_name().to_string_lossy().to_string();
                fileops::natural_sort_key(&a_name).cmp(&fileops::natural_sort_key(&b_name))
            })
            .into_iter();
        for entry in walk_iter.filter_map(|e| e.ok()) {
            if entry.file_type().is_dir() && has_images(entry.path()) {
                dirs_with_images.push(entry.path().to_path_buf());
            }
        }

        if dirs_with_images.is_empty() {
            dirs_with_images.push(dir);
        }

        dirs_to_browse = dirs_with_images;
    } else {
        archive_resume_base = None;
        dirs_to_browse = fileops::get_sorted_dirs(initial_dir);
    }

    // Resume state
    let mut cover_mode = true;
    let mut reading_mode = true;
    let mut force_single = false;

    if let Some(state) = resume_key.and_then(resume::get_resume_state) {
        cover_mode = state.cover_mode;
        reading_mode = state.reading_mode;
    }

    let mut dir_idx = 0;
    let mut img_idx = 0;
    let mut needs_redraw;
    let mut resumed = false;

    debug_log!(
        "Resume: dirs_to_browse ({} dirs): {:?}",
        dirs_to_browse.len(),
        dirs_to_browse
            .iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
    );

    // Try to find resume directory (restores last viewed chapter and page)
    if let Some(state) = resume_key.and_then(resume::get_resume_state) {
        debug_log!(
            "Resume: state loaded: dir_path={:?}, image_name={:?}, image_index={}",
            state.dir_path,
            state.image_name,
            state.image_index,
        );
        if let Some(resume_dir_idx) = resume::find_resume_dir_index(
            &dirs_to_browse,
            &state,
            is_archive,
            archive_resume_base.as_deref(),
        ) {
            debug_log!("Resume: found dir at index {}", resume_dir_idx);
            dir_idx = resume_dir_idx;
            let images = fileops::get_sorted_images(&dirs_to_browse[dir_idx]);
            img_idx = resume::find_resume_image_index(&images, &state);
            debug_log!("Resume: restored img_idx={}", img_idx);
            resumed = true;
        } else {
            debug_log!("Resume: dir not found in dirs_to_browse");
        }
    } else {
        debug_log!("Resume: no saved state found for key={:?}", resume_key);
    }

    debug_log!(
        "Resume: initial state: dir_idx={}, img_idx={}, resumed={}",
        dir_idx,
        img_idx,
        resumed,
    );

    // Start at the specified directory, not the first sibling.
    // This runs AFTER resume so that an explicit directory choice overrides history.
    // Only reset to page 0 if the user specified a different directory than the resume,
    // and only if resume did not successfully restore a position.
    //
    // If the user opened a specific chapter directory (has images directly),
    // always start at that directory. If resume restored the same directory,
    // keep the restored page position. If resume restored a different directory,
    // reset to page 0 of the opened directory.
    // For parent directories, respect the resume if it was successful.
    if !is_archive {
        let is_leaf_dir = !fileops::get_sorted_images(initial_dir).is_empty();
        if is_leaf_dir || !resumed {
            if let Some(pos) = dirs_to_browse.iter().position(|d| d == initial_dir) {
                debug_log!(
                    "Resume: override: initial_dir found at pos={}, dir_idx={}, is_leaf_dir={}",
                    pos,
                    dir_idx,
                    is_leaf_dir,
                );
                if pos != dir_idx {
                    dir_idx = pos;
                    img_idx = 0;
                    debug_log!("Resume: overridden to dir_idx={}, img_idx=0", dir_idx);
                } else {
                    // Same directory: keep the restored page position
                    debug_log!(
                        "Resume: same directory, keeping restored img_idx={}",
                        img_idx
                    );
                }
            } else {
                debug_log!("Resume: override: initial_dir not found in dirs_to_browse");
            }
        }
    }

    while dir_idx < dirs_to_browse.len() {
        needs_redraw = true;
        let mut dir_changed = false;
        let target_dir = &dirs_to_browse[dir_idx];

        // Click detection: column ranges for each toggle label and the status row
        let mut cover_label_cols: std::ops::Range<usize> = 0..0;
        let mut single_label_cols: std::ops::Range<usize> = 0..0;
        let mut mode_label_cols: std::ops::Range<usize> = 0..0;
        let mut prev_chapter_cols: std::ops::Range<usize> = 0..0;
        let mut next_chapter_cols: std::ops::Range<usize> = 0..0;
        let mut status_row: usize = 0;

        // Update last opened folder whenever the directory changes
        if let Ok(canon) = target_dir.canonicalize() {
            resume::set_last_opened_folder(&canon.to_string_lossy());
        }

        let images = fileops::get_sorted_images(target_dir);

        if images.is_empty() {
            dir_idx += 1;
            continue;
        }

        if img_idx >= images.len() {
            img_idx = images.len().saturating_sub(1);
        }

        while img_idx < images.len() {
            if needs_redraw {
                let _ = terminal::clear_screen();
                let (h, w) = terminal::get_terminal_size();

                let use_single =
                    display::should_display_single(&images, img_idx, cover_mode, force_single);
                let curr_right = &images[img_idx];
                let curr_left = if !use_single && img_idx + 1 < images.len() {
                    Some(&images[img_idx + 1])
                } else {
                    None
                };

                // Build status line: buttons first (left-aligned), then DIR info
                let dir_name = if is_archive {
                    archive_name.map(|n| n.to_string()).unwrap_or_default()
                } else {
                    target_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                };
                let file_info = if cover_mode && img_idx == 0 {
                    format!(
                        "Cover: {}",
                        curr_right
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    )
                } else if use_single {
                    format!(
                        "Single: {}",
                        curr_right
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    )
                } else {
                    let l_name = curr_left
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy()))
                        .unwrap_or_default();
                    format!(
                        "R: {} L: {l_name}",
                        curr_right
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    )
                };

                let mut status = String::new();

                let start = status.chars().count();
                status += if cover_mode { " [Cover]" } else { " [NoCover]" };
                cover_label_cols = start..status.chars().count();

                let start = status.chars().count();
                status += if force_single { " [Single]" } else { " [Multi]" };
                single_label_cols = start..status.chars().count();

                let start = status.chars().count();
                status += if reading_mode { " [Manga]" } else { " [Comic]" };
                mode_label_cols = start..status.chars().count();

                let start = status.chars().count();
                status += " [<前話]";
                prev_chapter_cols = start..status.chars().count();

                let start = status.chars().count();
                status += "[次話>]";
                next_chapter_cols = start..status.chars().count();

                status += &format!(" | DIR: {dir_name} | {file_info}");

                // Save resume state per-directory
                if let Ok(canon) = target_dir.canonicalize() {
                    let dir_key = canon.to_string_lossy().to_string();
                    resume::save_resume_state(
                        &dir_key,
                        target_dir,
                        &images,
                        img_idx,
                        is_archive,
                        archive_resume_base.as_deref(),
                        cover_mode,
                        reading_mode,
                    );
                }

                // Render
                renderer.clear();
                let _ = terminal::refresh_screen();

                if cover_mode && img_idx == 0 {
                    renderer.display_cover(curr_right, w, h);
                } else if use_single {
                    renderer.display_single(curr_right, w, h);
                } else if let Some(left) = curr_left {
                    if reading_mode {
                        // Manga (RTL): next page on LEFT, current page on RIGHT
                        renderer.display_spread(left, Some(curr_right), w, h);
                    } else {
                        // Comic (LTR): current page on LEFT, next page on RIGHT
                        renderer.display_spread(curr_right, Some(left), w, h);
                    }
                } else {
                    renderer.display_single(curr_right, w, h);
                }

                // Draw status line
                let status_offset = if renderer.is_kitty || renderer.is_wezterm {
                    1
                } else {
                    0
                };
                status_row = h.saturating_sub(1 + status_offset);
                let _ = terminal::draw_status(h, w, &status, status_offset);

                needs_redraw = false;
            }

            // Input handling
            let key = terminal::get_input(None);
            if key.is_none() {
                continue;
            }
            let key = key.unwrap();

            if key == terminal::InputKey::Resize {
                needs_redraw = true;
                continue;
            }

            let step = display::get_display_step(&images, img_idx, cover_mode, force_single);

            // Determine key mappings based on reading mode
            let (key_next, key_prev, key_turbo_next, key_turbo_prev) = if reading_mode {
                (
                    vec![
                        terminal::InputKey::Char('j'),
                        terminal::InputKey::Left,
                        terminal::InputKey::Enter,
                    ],
                    vec![
                        terminal::InputKey::Char('k'),
                        terminal::InputKey::Char('l'),
                        terminal::InputKey::Right,
                    ],
                    vec![terminal::InputKey::Char('J'), terminal::InputKey::ShiftLeft],
                    vec![
                        terminal::InputKey::Char('K'),
                        terminal::InputKey::Char('L'),
                        terminal::InputKey::ShiftRight,
                    ],
                )
            } else {
                (
                    vec![
                        terminal::InputKey::Char('j'),
                        terminal::InputKey::Right,
                        terminal::InputKey::Enter,
                    ],
                    vec![
                        terminal::InputKey::Char('k'),
                        terminal::InputKey::Char('l'),
                        terminal::InputKey::Left,
                    ],
                    vec![
                        terminal::InputKey::Char('J'),
                        terminal::InputKey::ShiftRight,
                    ],
                    vec![
                        terminal::InputKey::Char('K'),
                        terminal::InputKey::Char('L'),
                        terminal::InputKey::ShiftLeft,
                    ],
                )
            };

            let mut action: Option<&str> = None;

            if key_next.contains(&key) {
                let next_idx = img_idx + if cover_mode && img_idx == 0 { 1 } else { step };
                if next_idx >= images.len() {
                    if dir_idx + 1 < dirs_to_browse.len() {
                        dir_idx += 1;
                        img_idx = 0;
                        dir_changed = true;
                        break;
                    }
                } else {
                    img_idx = next_idx;
                }
                needs_redraw = true;
            } else if key_turbo_next.contains(&key) {
                img_idx =
                    std::cmp::min(images.len().saturating_sub(1), img_idx + crate::TURBO_STEP);
                needs_redraw = true;
            } else if key_prev.contains(&key) {
                if img_idx == 0 {
                    if dir_idx > 0 {
                        dir_idx -= 1;
                        img_idx = usize::MAX; // Will be set to last page
                        dir_changed = true;
                        break;
                    }
                } else {
                    img_idx = display::get_previous_page_index(
                        &images,
                        img_idx,
                        cover_mode,
                        force_single,
                    );
                }
                needs_redraw = true;
            } else if key_turbo_prev.contains(&key) {
                img_idx = img_idx.saturating_sub(crate::TURBO_STEP);
                needs_redraw = true;
            } else if key == terminal::InputKey::Char('0') {
                img_idx = 0;
                needs_redraw = true;
            } else if let terminal::InputKey::Char(c @ '1'..='9') = key {
                let percent = c.to_digit(10).unwrap() as usize * 10;
                img_idx = display::get_progress_index(images.len(), percent);
                needs_redraw = true;
            } else if key == terminal::InputKey::Char('c') {
                cover_mode = !cover_mode;
                img_idx = 0;
                needs_redraw = true;
            } else if key == terminal::InputKey::Char('r') {
                reading_mode = !reading_mode;
                needs_redraw = true;
            } else if key == terminal::InputKey::Char('s') {
                force_single = !force_single;
                needs_redraw = true;
            } else if key == terminal::InputKey::Char(',') {
                if dir_idx + 1 < dirs_to_browse.len() {
                    dir_idx += 1;
                    img_idx = 0;
                    dir_changed = true;
                    break;
                }
                needs_redraw = true;
            } else if key == terminal::InputKey::Char('.') {
                if dir_idx > 0 {
                    dir_idx -= 1;
                    img_idx = 0;
                    dir_changed = true;
                    break;
                }
                needs_redraw = true;
            } else if key == terminal::InputKey::Char('q')
                || key == terminal::InputKey::Char('Q')
                || key == terminal::InputKey::Char('h')
            {
                // Save resume state before quitting
                if let Some(key) = resume_key {
                    resume::save_resume_state(
                        key,
                        target_dir,
                        &images,
                        img_idx,
                        is_archive,
                        archive_resume_base.as_deref(),
                        cover_mode,
                        reading_mode,
                    );
                }
                return Ok(());
            } else if let terminal::InputKey::MouseLeft(row, col) = key {
                let (h, w) = terminal::get_terminal_size();
                let col_usize = col as usize;

                if row as usize == status_row && cover_label_cols.contains(&col_usize) {
                    cover_mode = !cover_mode;
                    img_idx = 0;
                    needs_redraw = true;
                } else if row as usize == status_row && single_label_cols.contains(&col_usize) {
                    force_single = !force_single;
                    needs_redraw = true;
                } else if row as usize == status_row && mode_label_cols.contains(&col_usize) {
                    reading_mode = !reading_mode;
                    needs_redraw = true;
                } else if row as usize == status_row && next_chapter_cols.contains(&col_usize) {
                    // 「,」キーと同じ: 次のディレクトリ(章)へ
                    if dir_idx + 1 < dirs_to_browse.len() {
                        dir_idx += 1;
                        img_idx = 0;
                        dir_changed = true;
                        break;
                    }
                } else if row as usize == status_row && prev_chapter_cols.contains(&col_usize) {
                    // 「.」キーと同じ: 前のディレクトリ(章)へ
                    if dir_idx > 0 {
                        dir_idx -= 1;
                        img_idx = 0;
                        dir_changed = true;
                        break;
                    }
                } else if terminal::is_mouse_in_image_area(row, col, h as u16, w as u16) {
                    action = Some("next");
                }
            } else if let terminal::InputKey::MouseRight(row, col) = key {
                let (h, w) = terminal::get_terminal_size();
                if terminal::is_mouse_in_image_area(row, col, h as u16, w as u16) {
                    action = Some("prev");
                }
            } else if let terminal::InputKey::MouseMiddle(_, _) = key {
                // Save resume state before quitting
                if let Some(key) = resume_key {
                    resume::save_resume_state(
                        key,
                        target_dir,
                        &images,
                        img_idx,
                        is_archive,
                        archive_resume_base.as_deref(),
                        cover_mode,
                        reading_mode,
                    );
                }
                return Ok(());
            } else if key == terminal::InputKey::Escape {
                // ESC sequence - handled by terminal module
            }

            // Handle mouse actions
            if let Some(action) = action {
                match action {
                    "next" => {
                        let next_idx = img_idx + if cover_mode && img_idx == 0 { 1 } else { step };
                        if next_idx >= images.len() {
                            if dir_idx + 1 < dirs_to_browse.len() {
                                dir_idx += 1;
                                img_idx = 0;
                                dir_changed = true;
                                break;
                            }
                        } else {
                            img_idx = next_idx;
                        }
                        needs_redraw = true;
                    }
                    "prev" => {
                        if img_idx == 0 {
                            if dir_idx > 0 {
                                dir_idx -= 1;
                                img_idx = usize::MAX;
                                dir_changed = true;
                                break;
                            }
                        } else {
                            img_idx = display::get_previous_page_index(
                                &images,
                                img_idx,
                                cover_mode,
                                force_single,
                            );
                        }
                        needs_redraw = true;
                    }
                    _ => {}
                }
            }
        }

        // Post-loop: handle directory transitions
        if dir_changed {
            // User-initiated directory change (',' / '.' / next-at-end / prev-at-start)
            // dir_idx already updated, just handle sentinel values
            if img_idx == usize::MAX {
                let new_images = fileops::get_sorted_images(&dirs_to_browse[dir_idx]);
                img_idx = new_images.len().saturating_sub(1);
            }
            // Otherwise img_idx is already correct (0 for next, or set above)
        } else if img_idx < images.len() {
            // Natural end of inner loop (all images viewed)
            dir_idx += 1;
            img_idx = 0;
        } else if img_idx == usize::MAX {
            // Previous volume sentinel (shouldn't reach here without dir_changed,
            // but handle defensively)
            let new_images = fileops::get_sorted_images(&dirs_to_browse[dir_idx]);
            img_idx = new_images.len().saturating_sub(1);
        }
    }

    renderer.clear();
    println!("All files displayed.");

    Ok(())
}
