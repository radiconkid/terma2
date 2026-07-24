//! File operations
//!
//! Provides functions for sorting directories, sorting images,
//! extracting archives, and handling nested archives.

use std::path::{Path, PathBuf};
use std::collections::HashSet;

/// Natural sort key function (e.g., "page2" < "page10").
pub fn natural_sort_key(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut prev_is_digit = false;

    for ch in s.chars() {
        let is_digit = ch.is_ascii_digit();
        if is_digit != prev_is_digit && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }
        current.push(ch);
        prev_is_digit = is_digit;
    }
    if !current.is_empty() {
        parts.push(current);
    }

    // Sort: numeric parts as integers (zero-padded), text parts as lowercase
    parts
        .into_iter()
        .map(|p| {
            if p.chars().all(|c| c.is_ascii_digit()) {
                format!("{:0>20}", p) // zero-pad for numeric comparison
            } else {
                p.to_lowercase()
            }
        })
        .collect()
}

/// Get sorted sibling directories of the given path.
pub fn get_sorted_dirs(initial_dir: &Path) -> Vec<PathBuf> {
    let parent = initial_dir.parent().unwrap_or(initial_dir);
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(parent) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect(),
        Err(_) => return vec![initial_dir.to_path_buf()],
    };
    dirs.sort_by(|a, b| {
        let a_name = a.file_name().unwrap_or_default().to_string_lossy();
        let b_name = b.file_name().unwrap_or_default().to_string_lossy();
        natural_sort_key(&a_name).cmp(&natural_sort_key(&b_name))
    });
    dirs
}

/// Supported image file extensions.
const IMAGE_EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".avif"];

/// Get sorted image files from a directory.
pub fn get_sorted_images(target_dir: &Path) -> Vec<PathBuf> {
    let extensions: HashSet<&str> = IMAGE_EXTENSIONS.iter().cloned().collect();
    // Priority order for duplicate stems (lower number = higher priority)
    let priority = |ext: &str| -> u8 {
        match ext {
            ".jpg" | ".jpeg" => 0,
            ".png" => 1,
            ".gif" => 2,
            ".webp" => 3,
            ".bmp" => 4,
            ".avif" => 5,
            _ => 99,
        }
    };

    let images: Vec<PathBuf> = match std::fs::read_dir(target_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().map(|t| t.is_file()).unwrap_or(false)
                    && e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| {
                            let ext_lower = format!(".{}", ext.to_lowercase());
                            extensions.contains(ext_lower.as_str())
                        })
                        .unwrap_or(false)
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => return vec![],
    };

    // Deduplicate by stem: keep the highest priority file for each stem
    let mut best: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
    for img in images {
        if let Some(stem) = img.file_stem().and_then(|s| s.to_str()) {
            let stem_lower = stem.to_lowercase();
            let ext = img
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{}", e.to_lowercase()))
                .unwrap_or_default();
            let should_replace = match best.get(&stem_lower) {
                Some(existing) => {
                    let existing_ext = existing
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{}", e.to_lowercase()))
                        .unwrap_or_default();
                    priority(&ext) < priority(&existing_ext)
                }
                None => true,
            };
            if should_replace {
                best.insert(stem_lower, img);
            }
        }
    }

    let mut result: Vec<PathBuf> = best.into_values().collect();
    result.sort_by(|a, b| {
        let a_name = a.file_name().unwrap_or_default().to_string_lossy();
        let b_name = b.file_name().unwrap_or_default().to_string_lossy();
        natural_sort_key(&a_name).cmp(&natural_sort_key(&b_name))
    });
    result
}

/// Extract an archive file to the given directory.
/// Supports ZIP/CBZ, RAR/CBR (via external unrar/7z), and TAR.
pub fn extract_archive(archive_path: &Path, extract_to: &Path) -> bool {
    // ZIP / CBZ
    if let Ok(file) = std::fs::File::open(archive_path) {
        let mut reader = std::io::BufReader::new(file);
        if zip::ZipArchive::new(&mut reader).is_ok() {
            if let Ok(mut archive) = zip::ZipArchive::new(&mut reader) {
                if archive.extract(extract_to).is_ok() {
                    return true;
                }
            }
        }
    }

    // RAR / CBR
    let is_rar = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase() == "rar" || e.to_lowercase() == "cbr")
        .unwrap_or(false);
    if is_rar {
        // Try unrar
        if let Ok(unrar_path) = which::which("unrar") {
            let result = std::process::Command::new(unrar_path)
                .args([
                    "x",
                    "-y",
                    &archive_path.canonicalize().unwrap_or_else(|_| archive_path.to_path_buf()).to_string_lossy(),
                    &format!("{}/", extract_to.canonicalize().unwrap_or_else(|_| extract_to.to_path_buf()).to_string_lossy()),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if let Ok(status) = result {
                if status.success() {
                    return true;
                }
            }
        }
        // Try 7z
        for cmd in &["7z", "7za"] {
            if let Ok(sevenz_path) = which::which(cmd) {
                let result = std::process::Command::new(sevenz_path)
                    .args([
                        "x",
                        "-y",
                        &format!("-o{}", extract_to.canonicalize().unwrap_or_else(|_| extract_to.to_path_buf()).to_string_lossy()),
                        &archive_path.canonicalize().unwrap_or_else(|_| archive_path.to_path_buf()).to_string_lossy(),
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                if let Ok(status) = result {
                    if status.success() {
                        return true;
                    }
                }
            }
        }
    }

    // TAR
    if let Ok(file) = std::fs::File::open(archive_path) {
        let reader = std::io::BufReader::new(file);
        let mut archive = tar::Archive::new(reader);
        if archive.unpack(extract_to).is_ok() {
            return true;
        }
    }

    false
}

/// Extract nested archive files recursively.
pub fn extract_nested_archives(root_dir: &Path) {
    let archive_exts: HashSet<&str> = [".zip", ".cbz", ".rar", ".cbr"].iter().cloned().collect();
    let mut found = true;
    while found {
        found = false;
        let entries: Vec<PathBuf> = match walkdir::WalkDir::new(root_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_path_buf())
            .collect()
        {
            entries => entries,
        };
        for arch in entries {
            if let Some(ext) = arch.extension().and_then(|e| e.to_str()) {
                let ext_lower = format!(".{}", ext.to_lowercase());
                if archive_exts.contains(ext_lower.as_str()) {
                    if let Some(stem) = arch.file_stem() {
                        let nested_dir = root_dir.join(stem);
                        let _ = std::fs::create_dir_all(&nested_dir);
                        if extract_archive(&arch, &nested_dir) {
                            let _ = std::fs::remove_file(&arch);
                            found = true;
                        }
                    }
                }
            }
        }
    }
}

/// Check if a path is an archive file.
#[allow(dead_code)]
pub fn is_archive_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "zip" | "cbz" | "rar" | "cbr" | "tar" | "gz" | "tgz" | "bz2" | "tbz" | "xz" | "txz"
    )
}

