//! CLI output styling module
//!
//! Provides unified color and formatting styles for CLI output.

use crossterm::{
    ExecutableCommand,
    style::{Color, Print, Stylize, style},
};
use std::io::stdout;

/// CLI theme colors
pub struct CliTheme;

impl CliTheme {
    /// Success color (green)
    pub const SUCCESS: Color = Color::Green;
    /// Warning color (yellow)
    pub const WARNING: Color = Color::Yellow;
    /// Error color (red)
    pub const ERROR: Color = Color::Red;
    /// Hint color (dark gray)
    pub const HINT: Color = Color::DarkGrey;
    /// Accent color (cyan)
    pub const ACCENT: Color = Color::Cyan;
}

/// Print a separator line
pub fn print_separator() {
    let _ = stdout().execute(Print(&format!("{}\n", "─".repeat(60))));
}

/// Print a title
pub fn print_title(title: &str) {
    let _ = stdout().execute(Print(title.bold().stylize()));
    let _ = stdout().execute(Print("\n\n"));
}

/// Print warning message
pub fn print_warning(msg: &str) {
    let _ = stdout().execute(Print(style("⚠ ").with(CliTheme::WARNING).bold()));
    let _ = stdout().execute(Print(format!("{}\n", msg)));
}

/// Print error message
pub fn print_error(msg: &str) {
    let _ = stdout().execute(Print(style("✗ ").with(CliTheme::ERROR).bold()));
    let _ = stdout().execute(Print(format!("{}\n", msg)));
}

/// Print hint message
pub fn print_hint(msg: &str) {
    let _ = stdout().execute(Print(style("→ ").with(CliTheme::HINT)));
    let _ = stdout().execute(Print(format!("{}\n", msg)));
}

/// Print key-value pair
pub fn print_key_value(key: &str, value: &str, value_color: Option<Color>) {
    let key_styled = style(key).with(CliTheme::HINT);
    let value_styled = match value_color {
        Some(color) => style(value).with(color),
        None => style(value).bold(),
    };
    let _ = stdout().execute(Print("  "));
    let _ = stdout().execute(Print(key_styled));
    let _ = stdout().execute(Print(": "));
    let _ = stdout().execute(Print(value_styled));
    let _ = stdout().execute(Print("\n"));
}

/// Print statistics item
pub fn print_stat(key: &str, value: &str, color: Color) {
    let key_styled = style(key).with(CliTheme::HINT);
    let value_styled = style(value).with(color).bold();
    let _ = stdout().execute(Print("  "));
    let _ = stdout().execute(Print(key_styled));
    let _ = stdout().execute(Print(": "));
    let _ = stdout().execute(Print(value_styled));
    let _ = stdout().execute(Print("\n"));
}

/// Print processing result line
pub fn print_result(status_icon: &str, status_color: Color, source: &str, dest_or_msg: &str) {
    let icon_styled = style(status_icon).with(status_color).bold();
    let source_styled = style(source).italic();
    let msg_styled = style(dest_or_msg).with(CliTheme::HINT);

    let _ = stdout().execute(Print("  "));
    let _ = stdout().execute(Print(icon_styled));
    let _ = stdout().execute(Print(" "));
    let _ = stdout().execute(Print(source_styled));
    let _ = stdout().execute(Print(" "));
    let _ = stdout().execute(Print(msg_styled));
    let _ = stdout().execute(Print("\n"));
}

/// Print log file path
pub fn print_log_path(path: &str) {
    let _ = stdout().execute(Print("\n"));
    let _ = stdout().execute(Print(style("  📁 ").with(CliTheme::ACCENT)));
    let _ = stdout().execute(Print(style("Log file: ").with(CliTheme::HINT)));
    let _ = stdout().execute(Print(format!("{}\n", path)));
}

/// Print blank line
pub fn print_blank() {
    let _ = stdout().execute(Print("\n"));
}
