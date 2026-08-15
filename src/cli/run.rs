//! CLI 运行编排与结果打印

use crate::cli::output;
use crate::cli::rename::run_unify_cli;
use crate::cli::{
    get_executable_dir, get_log_path, load_config, setup_file_only_logging, setup_logging,
    validate_config,
};
use anyhow::Result;
use chrono::Local;
use clap::Parser;
use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{error, info};

/// Convenience macro for translation
macro_rules! t {
    ($key:expr) => {
        rust_i18n::t!($key)
    };
    ($key:expr, $($tt:tt)*) => {
        rust_i18n::t!($key, $($tt)*)
    };
}

/// Run in interactive mode with Ratatui TUI
pub fn run_interactive_mode() -> Result<()> {
    // Get executable directory first for log path
    let exe_dir = get_executable_dir()?;
    let log_dir = exe_dir.join("Log");
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let log_path = log_dir.join(format!("Interactive_{}.log", timestamp));

    // Setup file-only logging before TUI starts
    let _guard = setup_file_only_logging(&log_path)?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Gallery Sorter starting in interactive mode"
    );

    let mut wizard = crate::tui::TuiApp::new()?;
    wizard.set_log_path(log_path.clone());

    // Run configuration wizard (processing happens within TUI)
    match wizard.run()? {
        Some(_) => {
            // Config was completed and processing ran within TUI
            info!(log_file = %log_path.display(), "Interactive session complete");
        }
        None => {
            // User cancelled
            info!("User cancelled interactive mode");
        }
    };

    // Log path is already displayed on the TUI summary screen

    Ok(())
}

/// Run in standard CLI mode
pub fn run_cli_mode() -> Result<()> {
    use crate::cli::Cli;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Get the executable directory for Config and Log directories
    let exe_dir = get_executable_dir()?;

    // Determine log file path based on config file or timestamp
    let log_path = get_log_path(&exe_dir, &cli);

    // 先加载配置以决定日志级别
    let (config, config_path) = load_config(&cli, &exe_dir)?;

    // Setup logging
    let _guard = setup_logging(&cli, &config, &log_path)?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Gallery Sorter starting"
    );

    if let Some(path) = config_path.as_ref() {
        info!(config_file = %path.display(), "Configuration loaded from file");
    }

    let verbose = cli.verbose || config.verbose;
    if verbose {
        info!(?config, "Configuration loaded");
    }

    let dry_run = config.dry_run;

    // Log to file location
    info!(log_file = %log_path.display(), "Log file location");

    // 文件名统一化模式：跳过归档流水线的校验（output 仅用于未修改列表）
    if config.unify_filenames {
        return run_unify_cli(config, &log_path);
    }

    // Validate configuration
    validate_config(&config)?;

    // 优雅取消：SIGINT/Ctrl+C → 置位取消标志 → 保存 state 后退出（ADR-0003）
    let cancel = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler(build_cancel_handler(cancel.clone()))
        .map_err(|e| anyhow::anyhow!("Error setting Ctrl-C handler: {}", e))?;

    // Create and run processor
    let mut processor = crate::process::Processor::new_with_cancel(config, cancel)?;

    // CLI 进度：stderr 为 TTY 或 --verbose 时输出单行进度，管道/重定向静默
    let show_progress = should_show_progress(verbose, std::io::stderr().is_terminal());
    let progress_done = Arc::new(AtomicBool::new(false));
    let progress_thread = if show_progress {
        let stats = processor.stats_arc();
        let total = processor.total_files_count().unwrap_or(0);
        let done = progress_done.clone();
        Some(std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let processed = stats.processed.load(Ordering::Relaxed);
                eprint!("{}", format_progress(processed, total));
                std::thread::sleep(Duration::from_millis(100));
            }
        }))
    } else {
        None
    };

    let run_result = processor.run();

    if let Some(thread) = progress_thread {
        progress_done.store(true, Ordering::Relaxed);
        let _ = thread.join();
        eprintln!();
    }

    match run_result {
        Ok(results) => {
            use output::*;

            // Store translations to avoid temporary value issues
            let stat_processed = t!("stat_processed");
            let stat_skipped = t!("stat_skipped");
            let stat_duplicates = t!("stat_duplicates");
            let stat_failed = t!("stat_failed");

            // Print summary header
            print_separator();
            print_title(&t!("cli_processing_complete"));
            print_separator();

            if processor.was_cancelled() {
                print_warning(&t!("cli_processing_cancelled"));
                print_blank();
            }

            // Get stats
            let stats = processor.stats();
            let processed = stats.processed.load(std::sync::atomic::Ordering::Relaxed);
            let skipped = stats.skipped.load(std::sync::atomic::Ordering::Relaxed);
            let duplicates = stats.duplicates.load(std::sync::atomic::Ordering::Relaxed);
            let failed_count = stats.failed.load(std::sync::atomic::Ordering::Relaxed);

            // Print stats with colors
            print_blank();
            print_stat(&stat_processed, &processed.to_string(), CliTheme::SUCCESS);
            print_stat(&stat_skipped, &skipped.to_string(), CliTheme::WARNING);
            print_stat(&stat_duplicates, &duplicates.to_string(), CliTheme::ACCENT);
            print_stat(&stat_failed, &failed_count.to_string(), CliTheme::ERROR);
            print_blank();

            // Store translations for results
            let already_processed = t!("already_processed");
            let duplicate_of = t!("duplicate_of");
            let unknown_error = t!("unknown_error");

            // Print detailed results if verbose
            if verbose {
                print_separator();
                print_hint(&t!("cli_detailed_results"));
                print_blank();

                for result in &results {
                    match result.status {
                        crate::process::ProcessingStatus::Success => {
                            let dest = result
                                .destination
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            print_result(
                                "✓",
                                CliTheme::SUCCESS,
                                &result.source.display().to_string(),
                                &format!("→ {}", dest),
                            );
                        }
                        crate::process::ProcessingStatus::Skipped => {
                            print_result(
                                "⊘",
                                CliTheme::WARNING,
                                &result.source.display().to_string(),
                                &already_processed,
                            );
                        }
                        crate::process::ProcessingStatus::Duplicate => {
                            let dest = result
                                .destination
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            print_result(
                                "≡",
                                CliTheme::ACCENT,
                                &result.source.display().to_string(),
                                &format!("{}: {}", duplicate_of, dest),
                            );
                        }
                        crate::process::ProcessingStatus::Failed => {
                            let error_msg = result.error.as_deref().unwrap_or(&unknown_error);
                            print_result(
                                "✗",
                                CliTheme::ERROR,
                                &result.source.display().to_string(),
                                error_msg,
                            );
                        }
                        crate::process::ProcessingStatus::DryRun => {
                            let dest = result
                                .destination
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            print_result(
                                "~",
                                CliTheme::ACCENT,
                                &result.source.display().to_string(),
                                &format!("→ {}", dest),
                            );
                        }
                        crate::process::ProcessingStatus::Cancelled => {
                            // 取消时未开始处理的文件不打印明细（T11 处理取消横幅）
                        }
                    }
                }
            }

            // Report failed files summary
            let failed_items: Vec<_> = results
                .iter()
                .filter(|r| r.status == crate::process::ProcessingStatus::Failed)
                .collect();

            if !failed_items.is_empty() {
                print_separator();
                print_error(&format!(
                    "{} {} {}",
                    t!("cli_failed_files"),
                    failed_items.len(),
                    t!("files")
                ));
                print_blank();
                for result in &failed_items {
                    let error_msg = result.error.as_deref().unwrap_or(&unknown_error);
                    print_key_value(
                        &result.source.display().to_string(),
                        error_msg,
                        Some(CliTheme::ERROR),
                    );
                }
            }

            if dry_run {
                print_separator();
                print_warning(&t!("cli_dry_run_notice"));
            }

            // Print log file path
            print_separator();
            print_log_path(&log_path.display().to_string());

            info!(log_file = %log_path.display(), "Processing complete. Log saved to");

            Ok(())
        }
        Err(e) => {
            error!(error = %e, "Processing failed");
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// 构建 Ctrl+C 取消处理器（只置位标志，不做进程级退出）
pub(crate) fn build_cancel_handler(cancel: Arc<AtomicBool>) -> impl Fn() {
    move || {
        cancel.store(true, Ordering::Relaxed);
    }
}

/// 是否显示 CLI 进度（stderr 为 TTY 或 --verbose）
pub(crate) fn should_show_progress(verbose: bool, stderr_is_tty: bool) -> bool {
    verbose || stderr_is_tty
}

/// 格式化单行进度：`\r 45% (1234/2743)`
pub(crate) fn format_progress(processed: usize, total: usize) -> String {
    let percent = processed
        .checked_mul(100)
        .and_then(|n| n.checked_div(total))
        .unwrap_or(0)
        .min(100);
    format!("\r {}% ({}/{})", percent, processed, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_cancel_handler_sets_flag() {
        let cancel = Arc::new(AtomicBool::new(false));
        let handler = build_cancel_handler(cancel.clone());

        assert!(!cancel.load(Ordering::Relaxed));
        handler();
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn test_should_show_progress() {
        assert!(!should_show_progress(false, false));
        assert!(should_show_progress(true, false));
        assert!(should_show_progress(false, true));
        assert!(should_show_progress(true, true));
    }

    #[test]
    fn test_format_progress() {
        assert_eq!(format_progress(0, 0), "\r 0% (0/0)");
        assert_eq!(format_progress(0, 100), "\r 0% (0/100)");
        assert_eq!(format_progress(1, 3), "\r 33% (1/3)");
        assert_eq!(format_progress(1234, 2743), "\r 44% (1234/2743)");
        assert_eq!(format_progress(2743, 2743), "\r 100% (2743/2743)");
        // 防止统计异常时百分比超过 100
        assert_eq!(format_progress(999, 100), "\r 100% (999/100)");
    }
}
