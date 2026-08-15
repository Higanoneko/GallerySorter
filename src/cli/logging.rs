//! CLI 日志初始化

use crate::cli::Cli;
use crate::config::Config;
use anyhow::Result;
use chrono::Local;
use std::path::{Path, PathBuf};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Determine the log file path based on config file or timestamp
pub fn get_log_path(exe_dir: &Path, cli: &Cli) -> PathBuf {
    let log_dir = exe_dir.join("Log");
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");

    if let Some(config_name) = cli.config_name() {
        let config_log_dir = log_dir.join(&config_name);
        let log_filename = format!("{}_{}.log", config_name, timestamp);
        config_log_dir.join(log_filename)
    } else {
        let log_filename = format!("CLIRun_{}.log", timestamp);
        log_dir.join(log_filename)
    }
}

/// Setup logging for CLI mode (file + console)
pub fn setup_logging(cli: &Cli, config: &Config, log_path: &Path) -> Result<WorkerGuard> {
    let verbose = cli.verbose || config.verbose;
    let file_level = if verbose { Level::DEBUG } else { Level::INFO };
    let console_level = Level::WARN;

    let file_filter = EnvFilter::builder()
        .with_default_directive(file_level.into())
        .from_env_lossy();
    let console_filter = EnvFilter::builder()
        .with_default_directive(console_level.into())
        .from_env_lossy();

    let (non_blocking, guard) = create_log_writer(log_path)?;

    if cli.json_log {
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .json()
                    .with_ansi(false)
                    .with_writer(non_blocking)
                    .with_filter(file_filter),
            )
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_filter(console_filter),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking)
                    .with_filter(file_filter),
            )
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_filter(console_filter),
            )
            .init();
    }

    Ok(guard)
}

/// Setup logging for interactive mode (file only, no console)
pub fn setup_file_only_logging(log_path: &Path) -> Result<WorkerGuard> {
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();

    let (non_blocking, guard) = create_log_writer(log_path)?;

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_ansi(false).with_writer(non_blocking))
        .init();

    Ok(guard)
}

/// 创建日志输出并确保目录存在
pub fn create_log_writer(
    log_path: &Path,
) -> Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)?;

    Ok(tracing_appender::non_blocking(file))
}
