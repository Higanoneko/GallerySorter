//! CLI 文件名统一化（rename）运行编排

use crate::cli::output::{
    CliTheme, print_blank, print_hint, print_key_value, print_log_path, print_result,
    print_separator, print_stat, print_title, print_warning,
};
use crate::cli::run::{build_cancel_handler, format_progress, should_show_progress};
use crate::config::Config;
use crate::rename::{RenameStatus, Renamer, write_unmodified_list};
use anyhow::Result;
use std::io::IsTerminal;
use std::path::Path;
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

/// 运行文件名统一化 CLI 模式
///
/// 扫描输入目录 → 基于 EXIF/FFprobe 元数据原地重命名为
/// `YYYYMMDD_HHMMSS.ext`；无元数据的文件保持原名并写入未修改列表。
pub fn run_unify_cli(config: Config, log_path: &Path) -> Result<()> {
    // 与 validate_config 一致的缺失目录警告（rename 模式的 output 仅用于报告）
    for input_dir in &config.input_dirs {
        if !input_dir.exists() {
            eprintln!("{} {}", t!("cli_input_dir_not_exist"), input_dir.display());
        }
    }

    // 优雅取消：SIGINT/Ctrl+C → 置位取消标志（与归档流水线一致）
    let cancel = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler(build_cancel_handler(cancel.clone()))
        .map_err(|e| anyhow::anyhow!("Error setting Ctrl-C handler: {}", e))?;

    let mut renamer = Renamer::new_with_cancel(config.clone(), cancel)?;

    // CLI 进度：stderr 为 TTY 或 --verbose 时输出单行进度
    let show_progress = should_show_progress(config.verbose, std::io::stderr().is_terminal());
    let progress_done = Arc::new(AtomicBool::new(false));
    let progress_thread = if show_progress {
        let stats = renamer.stats_arc();
        let total = renamer.total_files_count().unwrap_or(0);
        let done = progress_done.clone();
        Some(std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let handled = stats.handled.load(Ordering::Relaxed);
                eprint!("{}", format_progress(handled, total));
                std::thread::sleep(Duration::from_millis(100));
            }
        }))
    } else {
        None
    };

    let run_result = renamer.run();

    if let Some(thread) = progress_thread {
        progress_done.store(true, Ordering::Relaxed);
        let _ = thread.join();
        eprintln!();
    }

    match run_result {
        Ok(results) => {
            let unmodified_list = config.get_unmodified_list_file();

            // 未修改列表（试运行不写文件，与归档流水线 dry-run 语义一致）
            if !config.dry_run {
                match write_unmodified_list(&unmodified_list, &results) {
                    Ok(count) => {
                        info!(
                            path = %unmodified_list.display(),
                            count,
                            "Wrote unmodified files list"
                        );
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to write unmodified files list");
                    }
                }
            }

            print_separator();
            print_title(&t!("unify_complete"));
            print_separator();

            if renamer.was_cancelled() {
                print_warning(&t!("unify_cancelled"));
                print_blank();
            }

            let stats = renamer.stats();
            let renamed = stats.renamed.load(Ordering::Relaxed);
            let already_unified = stats.already_unified.load(Ordering::Relaxed);
            let no_metadata = stats.no_metadata.load(Ordering::Relaxed);
            let failed_count = stats.failed.load(Ordering::Relaxed);

            print_blank();
            print_stat(
                &t!("unify_renamed"),
                &renamed.to_string(),
                CliTheme::SUCCESS,
            );
            print_stat(
                &t!("unify_already_unified"),
                &already_unified.to_string(),
                CliTheme::WARNING,
            );
            print_stat(
                &t!("unify_no_metadata"),
                &no_metadata.to_string(),
                CliTheme::HINT,
            );
            print_stat(
                &t!("unify_failed"),
                &failed_count.to_string(),
                CliTheme::ERROR,
            );
            print_blank();

            if !config.dry_run {
                let unmodified_count = results
                    .iter()
                    .filter(|r| matches!(r.status, RenameStatus::NoMetadata | RenameStatus::Failed))
                    .count();
                if unmodified_count > 0 {
                    print_key_value(
                        &t!("unify_unmodified_list_written"),
                        &unmodified_list.display().to_string(),
                        Some(CliTheme::ACCENT),
                    );
                    print_blank();
                } else {
                    print_hint(&t!("unify_no_unmodified"));
                    print_blank();
                }
            }

            // 详细结果（--verbose）
            if config.verbose {
                print_separator();
                print_hint(&t!("cli_detailed_results"));
                print_blank();

                for result in &results {
                    match result.status {
                        RenameStatus::Renamed | RenameStatus::DryRun => {
                            let icon = if result.status == RenameStatus::Renamed {
                                "✓"
                            } else {
                                "~"
                            };
                            let color = if result.status == RenameStatus::Renamed {
                                CliTheme::SUCCESS
                            } else {
                                CliTheme::ACCENT
                            };
                            let dest = result
                                .destination
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            print_result(
                                icon,
                                color,
                                &result.source.display().to_string(),
                                &format!("→ {}", dest),
                            );
                        }
                        RenameStatus::AlreadyUnified => {
                            print_result(
                                "⊘",
                                CliTheme::WARNING,
                                &result.source.display().to_string(),
                                &t!("unify_already_unified"),
                            );
                        }
                        RenameStatus::NoMetadata => {
                            print_result(
                                "⚠",
                                CliTheme::HINT,
                                &result.source.display().to_string(),
                                &t!("unify_no_metadata"),
                            );
                        }
                        RenameStatus::Failed => {
                            let unknown_error = t!("unknown_error");
                            let error_msg = result.error.as_deref().unwrap_or(&unknown_error);
                            print_result(
                                "✗",
                                CliTheme::ERROR,
                                &result.source.display().to_string(),
                                error_msg,
                            );
                        }
                        RenameStatus::Cancelled => {
                            // 取消时未处理的文件不打印明细
                        }
                    }
                }
            }

            if config.dry_run {
                print_separator();
                print_warning(&t!("unify_dry_run_notice"));
            }

            print_separator();
            print_log_path(&log_path.display().to_string());

            info!(log_file = %log_path.display(), "Filename unification complete");
            Ok(())
        }
        Err(e) => {
            error!(error = %e, "Filename unification failed");
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
