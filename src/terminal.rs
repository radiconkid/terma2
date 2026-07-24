//! Terminal detection and input handling
//!
//! Provides functions for detecting terminal capabilities (Sixel, Kitty, WezTerm)
//! and handling keyboard/mouse input.

use std::io::{self, Read, Write};
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

/// Terminal type detection
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TerminalType {
    /// Kitty terminal with native graphics protocol
    Kitty,
    /// WezTerm with native imgcat support
    WezTerm,
    /// Sixel-capable terminal (foot, mlterm, xterm, etc.)
    Sixel,
    /// Unknown terminal type
    Unknown,
}

/// Detect the terminal type based on environment variables.
pub fn detect_terminal_type() -> TerminalType {
    let term_program = std::env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_lowercase();
    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();

    // Kitty detection
    if term_program.contains("kitty")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
        || term.contains("xterm-kitty")
    {
        return TerminalType::Kitty;
    }

    // WezTerm detection
    if std::env::var("WEZTERM_PANE").is_ok()
        || std::env::var("WEZTERM_UNIX_SOCKET").is_ok()
        || term_program.contains("wezterm")
    {
        return TerminalType::WezTerm;
    }

    // Sixel detection
    if is_sixel_terminal() {
        return TerminalType::Sixel;
    }

    TerminalType::Unknown
}

/// Check if the terminal supports Sixel graphics.
fn is_sixel_terminal() -> bool {
    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();
    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default().to_lowercase();

    // Explicitly Sixel-capable terminals
    if term.starts_with("foot") || term == "mlterm" || term.contains("contour") {
        return true;
    }
    if term_program == "mintty" {
        return true;
    }
    if std::env::var("WT_SESSION").is_ok() {
        return true;
    }

    // xterm-compatible with truecolor
    if term.contains("xterm") && colorterm == "truecolor" {
        return true;
    }

    // Check if chafa is available as a fallback
    if which::which("chafa").is_ok() {
        return true;
    }

    false
}

/// Check if chafa is installed and usable.
pub fn is_chafa_usable() -> bool {
    which::which("chafa").is_ok()
}

/// Input key types (alias for app.rs compatibility)
pub type InputKey = InputEvent;

/// Input event types
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Character input
    Char(char),
    /// Arrow key
    Up,
    Down,
    Left,
    Right,
    /// Shift + Arrow
    ShiftLeft,
    ShiftRight,
    /// Enter key
    Enter,
    /// Escape key
    Escape,
    /// Mouse events
    MouseLeft,
    MouseRight,
    MouseMiddle,
    /// Resize event
    Resize,
    /// No input (timeout)
    None,
}

/// Read a single byte from stdin with timeout.
fn read_byte_with_timeout(timeout_ms: u64) -> Option<u8> {
    let mut buf = [0u8; 1];
    let mut stdin = io::stdin();

    // Use polling approach with a short sleep
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if stdin.read(&mut buf).ok()? > 0 {
            return Some(buf[0]);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// Read input from the terminal with optional timeout.
///
/// Returns an InputEvent. If timeout_ms is Some, returns InputEvent::None on timeout.
/// If timeout_ms is None, blocks indefinitely.
pub fn read_input(timeout_ms: Option<u64>) -> InputEvent {
    let mut buf = [0u8; 1];
    let mut stdin = io::stdin();

    // Set stdin to non-blocking if timeout is specified
    if let Some(timeout) = timeout_ms {
        let start = std::time::Instant::now();
        loop {
            if stdin.read(&mut buf).is_ok() && buf[0] != 0 {
                break;
            }
            if start.elapsed().as_millis() > timeout as u128 {
                return InputEvent::None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        // Blocking read
        if stdin.read(&mut buf).is_err() || buf[0] == 0 {
            return InputEvent::None;
        }
    }

    let ch = buf[0];

    // Ctrl+C
    if ch == 0x03 {
        std::process::exit(0);
    }

    // Escape sequences
    if ch == 0x1b {
        return parse_escape_sequence();
    }

    // Enter
    if ch == b'\r' || ch == b'\n' {
        return InputEvent::Enter;
    }

    // Regular character
    if ch.is_ascii() {
        return InputEvent::Char(ch as char);
    }

    InputEvent::None
}

/// Parse escape sequences for arrow keys, mouse events, etc.
fn parse_escape_sequence() -> InputEvent {
    // Read the next byte after ESC
    let Some(next) = read_byte_with_timeout(100) else {
        return InputEvent::Escape;
    };

    // CSI sequences: ESC [
    if next == b'[' {
        let mut seq = Vec::new();
        loop {
            let Some(b) = read_byte_with_timeout(50) else {
                break;
            };
            seq.push(b);
            // Terminal characters: letters or ~
            if b.is_ascii_alphabetic() || b == b'~' {
                break;
            }
        }

        if seq.is_empty() {
            return InputEvent::Escape;
        }

        let seq_str = String::from_utf8_lossy(&seq);

        // Arrow keys
        if seq_str == "A" {
            return InputEvent::Up;
        }
        if seq_str == "B" {
            return InputEvent::Down;
        }
        if seq_str == "C" {
            return InputEvent::Right;
        }
        if seq_str == "D" {
            return InputEvent::Left;
        }

        // Shift + Arrow: [1;2D, [1;2C
        if seq_str == "1;2D" {
            return InputEvent::ShiftLeft;
        }
        if seq_str == "1;2C" {
            return InputEvent::ShiftRight;
        }

        // SGR mouse: [<Cb;Cx;CyM or m
        if seq_str.starts_with('<') && (seq.last() == Some(&b'M') || seq.last() == Some(&b'm')) {
            let body = &seq_str[1..seq_str.len() - 1]; // Remove '<' and 'M/m'
            let parts: Vec<&str> = body.split(';').collect();
            if parts.len() == 3 {
                if let (Ok(btn_code), Ok(_mx), Ok(_my)) =
                    (parts[0].parse::<u32>(), parts[1].parse::<u32>(), parts[2].parse::<u32>())
                {
                    let is_motion = btn_code & 32 != 0;
                    if !is_motion {
                        let btn = btn_code & 3;
                        let pressed = seq.last() == Some(&b'M');
                        if pressed {
                            return match btn {
                                0 => InputEvent::MouseLeft,
                                2 => InputEvent::MouseRight,
                                1 => InputEvent::MouseMiddle,
                                _ => InputEvent::None,
                            };
                        }
                    }
                }
            }
        }

        return InputEvent::Escape;
    }

    // Alt+key or other sequences
    InputEvent::Escape
}

/// Enable mouse reporting (SGR mode).
pub fn enable_mouse() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[?1000h\x1b[?1006h\x1b[?1007h")?;
    stdout.flush()?;
    Ok(())
}

/// Disable mouse reporting.
pub fn disable_mouse() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[?1000l\x1b[?1006l")?;
    stdout.flush()?;
    Ok(())
}

/// Hide the cursor.
pub fn hide_cursor() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[?25l")?;
    stdout.flush()?;
    Ok(())
}

/// Show the cursor.
pub fn show_cursor() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[?25h")?;
    stdout.flush()?;
    Ok(())
}

/// Clear the screen.
pub fn clear_screen() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[2J\x1b[H")?;
    stdout.flush()?;
    Ok(())
}

/// Get terminal size (rows, columns).
pub fn get_terminal_size() -> (usize, usize) {
    let size = terminal_size::terminal_size();
    match size {
        Some((terminal_size::Width(w), terminal_size::Height(h))) => (h as usize, w as usize),
        None => (24, 80),
    }
}

/// Write status text to a specific line.
pub fn draw_status(lines: usize, cols: usize, text: &str, offset: usize) -> io::Result<()> {
    let status_line = if lines >= 2 + offset {
        lines - 2 + offset
    } else {
        1
    };
    let max_width = if cols >= 2 { cols - 2 } else { 0 };
    let truncated = truncate_by_width(text, max_width);
    let mut stdout = io::stdout();
    write!(
        stdout,
        "\r\x1b[{};1H\x1b[K{:<width$}",
        status_line,
        truncated,
        width = max_width
    )?;
    stdout.flush()?;
    Ok(())
}

/// Truncate text to fit within max_width columns, accounting for wide characters.
fn truncate_by_width(text: &str, max_width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if text.width() <= max_width {
        return text.to_string();
    }
    let mut result = String::new();
    let mut width = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > max_width {
            break;
        }
        result.push(ch);
        width += w;
    }
    result
}

/// Initialize terminal for raw input mode.
pub fn init_terminal() -> io::Result<()> {
    enable_mouse()?;
    hide_cursor()?;
    Ok(())
}

/// Restore terminal to normal mode.
pub fn restore_terminal() -> io::Result<()> {
    disable_mouse()?;
    show_cursor()?;
    Ok(())
}

/// Get a single input event, blocking until one is available.
pub fn get_input(_timeout: Option<u64>) -> Option<InputKey> {
    let event = read_input(None);
    match event {
        InputEvent::None => None,
        other => Some(other),
    }
}

/// Refresh the screen (flush stdout).
pub fn refresh_screen() -> io::Result<()> {
    io::stdout().flush()?;
    Ok(())
}
