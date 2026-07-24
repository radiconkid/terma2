//! Resume state persistence
//!
//! Provides functions for saving and loading the last viewed position
//! to/from a JSON file (~/.terma_resume.json).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::debug_log;

/// Resume state for a single directory/archive
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeState {
    /// Name of the current image file
    pub image_name: String,
    /// Index of the current image
    pub image_index: usize,
    /// Whether this is an archive
    pub is_archive: bool,
    /// Cover mode state
    pub cover_mode: bool,
    /// Reading mode (true = Manga RTL, false = Comic LTR)
    pub reading_mode: bool,
    /// Relative directory path (for archives)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir_rel: Option<String>,
    /// Absolute directory path (for non-archives)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir_path: Option<String>,
}

/// Get the path to the resume file.
fn resume_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".terma_resume.json")
}

/// Load all resume data from the JSON file.
fn load_resume_data() -> std::collections::HashMap<String, ResumeState> {
    let path = resume_file_path();
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_default()
            }
            Err(e) => {
                debug_log!("Failed to load resume data: {e}");
                std::collections::HashMap::new()
            }
        }
    } else {
        std::collections::HashMap::new()
    }
}

/// Save all resume data to the JSON file.
fn save_resume_data(data: &std::collections::HashMap<String, ResumeState>) {
    let path = resume_file_path();
    match serde_json::to_string_pretty(data) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                debug_log!("Failed to save resume data: {e}");
            }
        }
        Err(e) => {
            debug_log!("Failed to serialize resume data: {e}");
        }
    }
}

/// Get the resume state for a specific key.
pub fn get_resume_state(resume_key: &str) -> Option<ResumeState> {
    let data = load_resume_data();
    data.get(resume_key).cloned()
}

/// Save the current state for a specific key.
pub fn save_resume_state(
    resume_key: &str,
    target_dir: &Path,
    images: &[PathBuf],
    img_idx: usize,
    is_archive: bool,
    archive_resume_base: Option<&Path>,
    cover_mode: bool,
    reading_mode: bool,
) {
    if images.is_empty() {
        return;
    }
    let safe_idx = img_idx.min(images.len().saturating_sub(1));
    let mut state = ResumeState {
        image_name: images[safe_idx]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        image_index: safe_idx,
        is_archive,
        cover_mode,
        reading_mode,
        dir_rel: None,
        dir_path: None,
    };

    if is_archive {
        if let Some(base) = archive_resume_base {
            let base_canon = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
            if let Ok(target_canon) = target_dir.canonicalize() {
                if let Ok(rel) = target_canon.strip_prefix(&base_canon) {
                    state.dir_rel = Some(rel.to_string_lossy().to_string());
                } else {
                    state.dir_rel = Some(".".to_string());
                }
            } else {
                state.dir_rel = Some(".".to_string());
            }
        }
    } else {
        if let Ok(canon) = target_dir.canonicalize() {
            state.dir_path = Some(canon.to_string_lossy().to_string());
        }
    }

    let mut data = load_resume_data();
    data.insert(resume_key.to_string(), state);
    save_resume_data(&data);
}

/// Find the directory index from resume state.
pub fn find_resume_dir_index(
    dirs_to_browse: &[PathBuf],
    state: &ResumeState,
    is_archive: bool,
    archive_resume_base: Option<&Path>,
) -> Option<usize> {
    if is_archive {
        let saved_rel = state.dir_rel.as_deref()?;
        let base = archive_resume_base?;
        let base_canon = base.canonicalize().ok()?;
        for (i, d) in dirs_to_browse.iter().enumerate() {
            if let Ok(d_canon) = d.canonicalize() {
                if let Ok(rel) = d_canon.strip_prefix(&base_canon) {
                    if rel.to_string_lossy() == saved_rel {
                        return Some(i);
                    }
                }
            }
        }
    } else {
        let saved_dir = state.dir_path.as_deref()?;
        let saved_path = Path::new(saved_dir);
        for (i, d) in dirs_to_browse.iter().enumerate() {
            if let Ok(canon) = d.canonicalize() {
                if canon == saved_path.canonicalize().unwrap_or_else(|_| saved_path.to_path_buf()) {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Find the image index from resume state.
pub fn find_resume_image_index(images: &[PathBuf], state: &ResumeState) -> usize {
    let image_name = &state.image_name;
    for (i, image) in images.iter().enumerate() {
        if let Some(name) = image.file_name() {
            if name == image_name.as_str() {
                return i;
            }
        }
    }
    state.image_index.min(images.len().saturating_sub(1))
}

