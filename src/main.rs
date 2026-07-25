//! TerMa - Terminal Manga Viewer
//!
//! A terminal-based manga/comic reader with support for:
//! - Image display via chafa (Sixel/Kitty), Kitty icat, or WezTerm imgcat
//! - Archive extraction (ZIP/CBZ, RAR/CBR, TAR)
//! - Resume state persistence
//! - Manga (RTL) and Comic (LTR) reading modes
//! - Cover mode, spread display, single page mode
//! - Mouse support (SGR mode)

mod app;
mod chafa_render;
mod display;
mod fileops;
mod image;
mod renderer;
mod resume;
mod terminal;

use anyhow::Result;
use std::path::PathBuf;
use std::sync::OnceLock;

const VERSION: &str = "1.0.4";
const TURBO_STEP: usize = 10;

/// Simple debug logger that writes to stderr when TERMA_DEBUG is set.
static TERMA_DEBUG_INIT: OnceLock<bool> = OnceLock::new();

fn is_debug_enabled() -> bool {
    *TERMA_DEBUG_INIT.get_or_init(|| {
        let enabled = std::env::var("TERMA_DEBUG").as_deref() == Ok("1");
        if enabled {
            eprintln!("[terma] TERMA_DEBUG=1 detected, logging to stderr");
        }
        enabled
    })
}

/// Debug log macro that only outputs when TERMA_DEBUG=1.
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::is_debug_enabled() {
            eprintln!($($arg)*);
        }
    };
}

fn main() -> Result<()> {
    // Initialize debug logging (checks TERMA_DEBUG)
    is_debug_enabled();

    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "-v" | "--version" => {
                println!("TerMa version {VERSION}");
                return Ok(());
            }
            "--help" => {
                print_help();
                return Ok(());
            }
            _ => {}
        }
    }

    let target_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        std::env::current_dir()?
    };

    app::run(target_path)?;
    Ok(())
}

fn print_help() {
    println!(
        r#"TerMa - Terminal Manga Viewer
Usage: terma [directory_or_archive]
Arguments:
  directory_or_archive    Manga directory or archive (zip/tar/cbz) to view (default: current directory)
  -v, --version           Show version information
  --help                  Show this help message
Environment:
  TERMA_DEBUG=1           Enable debug logging to stderr (redirect with 2>file.log)
Resume:
  Last viewed positions are saved to ~/.terma_resume.json and restored automatically.
Controls:
  j/Left/Enter  Next page
  k/l/Right     Previous page
  J/Shift+Left  Turbo Next ({TURBO_STEP} pages)
  K/Shift+Right Turbo Previous ({TURBO_STEP} pages)
  0            First page (cover)
  1-9          Jump to 10%-90% progress
  c            Toggle cover mode (first page as cover / start with spread)
  s            Toggle single page mode (force current page as single)
  r            Toggle reading mode (Manga RTL / Comic LTR)
  ,            Next volume
  .            Previous volume
  q/Q/h        Quit"#
    );
}
