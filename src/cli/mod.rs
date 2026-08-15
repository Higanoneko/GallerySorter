//! CLI 入口模块树
//!
//! 按职责拆分为配置加载/校验（config）、日志（logging）、运行编排（run）
//! 与输出样式（output）子模块；参数定义后续迁入 args 子模块。

use anyhow::Result;
use std::path::PathBuf;

pub mod args;
pub mod config;
pub mod logging;
pub mod output;
pub mod rename;
pub mod run;

pub use args::{Cli, should_run_interactive};
pub use config::{load_config, resolve_config_path, validate_config};
pub use logging::{create_log_writer, get_log_path, setup_file_only_logging, setup_logging};
pub use output::{
    CliTheme, print_blank, print_error, print_hint, print_key_value, print_log_path, print_result,
    print_separator, print_stat, print_title, print_warning,
};
pub use rename::run_unify_cli;
pub use run::{run_cli_mode, run_interactive_mode};

/// 获取可执行文件所在目录
pub fn get_executable_dir() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    Ok(exe_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".")))
}
