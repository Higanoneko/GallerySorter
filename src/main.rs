//! Gallery Sorter - Professional photo and video organization tool
//!
//! A CLI tool for organizing media files based on creation time with
//! intelligent time extraction from EXIF, video metadata, filenames,
//! and file system timestamps.

use anyhow::Result;
use gallery_sorter::cli::run::{run_cli_mode, run_interactive_mode};
use gallery_sorter::{init_locale, should_run_interactive};

// Initialize i18n for this binary
rust_i18n::i18n!("locales", fallback = "en");

fn main() -> Result<()> {
    // Initialize locale based on system settings
    init_locale();

    // Check if we should run in interactive mode
    if should_run_interactive() {
        return run_interactive_mode();
    }

    // Standard CLI mode
    run_cli_mode()
}
