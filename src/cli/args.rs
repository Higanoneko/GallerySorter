//! CLI 参数解析

use crate::config::{ClassificationRule, Config, FileOperation, MonthFormat, ProcessingMode};
use clap::Parser;
use std::path::PathBuf;

/// Gallery Sorter - Professional photo and video organization tool
///
/// Organizes your photos and videos by creation date with intelligent
/// time extraction from EXIF data, video metadata, filenames, and
/// file system timestamps.
#[derive(Parser, Debug)]
#[command(name = "gallery-sorter")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Path to configuration file (TOML format)
    ///
    /// When specified, settings from the config file are used as defaults.
    /// CLI arguments will override config file settings.
    #[arg(short = 'C', long)]
    pub config: Option<PathBuf>,

    /// Input directories to scan for media files
    #[arg(short = 'I', long, num_args = 1..)]
    pub input: Option<Vec<PathBuf>>,

    /// Output directory for organized files
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Processing mode:
    /// - full: Process all files, overwrite existing (default)
    /// - supplement: Skip files that already exist in target
    /// - incremental: Only process files newer than newest in target
    #[arg(short = 'M', long, value_enum)]
    pub mode: Option<ProcessingMode>,

    /// Classification rule for organizing files
    #[arg(short, long, value_enum)]
    pub classify: Option<ClassificationRule>,

    /// Month format for year-month classification
    #[arg(short = 'm', long, value_enum)]
    pub month_format: Option<MonthFormat>,

    /// Classify by file type (adds Images/Videos/RAW subdirectory)
    #[arg(long)]
    pub classify_by_type: bool,

    /// File operation mode
    #[arg(short = 'O', long, value_enum)]
    pub operation: Option<FileOperation>,

    /// Disable file deduplication
    #[arg(long)]
    pub no_deduplicate: bool,

    /// State file path for tracking processed files
    #[arg(long)]
    pub state_file: Option<PathBuf>,

    /// Number of threads for parallel processing (0 = auto)
    #[arg(short = 't', long)]
    pub threads: Option<usize>,

    /// Large file threshold in MB (files larger use sampled hashing)
    #[arg(long)]
    pub large_file_mb: Option<u64>,

    /// Dry run mode - show what would be done without doing it
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Unify filenames based on EXIF / FFprobe metadata (rename files in place)
    #[arg(long)]
    pub unify_filenames: bool,

    /// Path to the unmodified files list file (rename mode)
    #[arg(long)]
    pub unmodified_list: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Output log format as JSON
    #[arg(long)]
    pub json_log: bool,

    /// Force interactive TUI mode even when CLI arguments are present
    #[arg(short = 'i', long)]
    pub interactive: bool,
}

impl Cli {
    /// Get config file name (without extension) for log naming
    pub fn config_name(&self) -> Option<String> {
        self.config.as_ref().and_then(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
    }

    /// 将 CLI 参数覆盖到给定配置（CLI 参数优先）
    ///
    /// `to_config` 与 `merge_with_config` 共用的单一覆盖入口。
    /// 注意：这里不处理 `YearMonth` 特殊默认——那是 `to_config`
    /// 对“无配置文件路径”场景的有意行为差异。
    pub fn apply_overrides(&self, config: &mut Config) {
        // Override with CLI arguments if provided
        if let Some(ref inputs) = self.input {
            config.input_dirs = inputs.clone();
        }
        if let Some(ref output) = self.output {
            config.output_dir = output.clone();
        }
        if let Some(mode) = self.mode {
            config.processing_mode = mode;
        }
        if let Some(classify) = self.classify {
            config.classification = classify;
        }
        if let Some(month_format) = self.month_format {
            config.month_format = month_format;
        }
        if self.classify_by_type {
            config.classify_by_type = true;
        }
        if let Some(operation) = self.operation {
            config.operation = operation;
        }
        if self.no_deduplicate {
            config.deduplicate = false;
        }
        if let Some(ref state_file) = self.state_file {
            config.state_file = Some(state_file.clone());
        }
        if let Some(threads) = self.threads {
            config.threads = threads;
        }
        if let Some(large_file_mb) = self.large_file_mb {
            config.large_file_threshold = large_file_mb * 1024 * 1024;
        }
        if self.dry_run {
            config.dry_run = true;
        }
        if self.unify_filenames {
            config.unify_filenames = true;
        }
        if let Some(ref path) = self.unmodified_list {
            config.unmodified_list_file = Some(path.clone());
        }
        if self.verbose {
            config.verbose = true;
        }
    }

    /// Merge CLI arguments with config from file
    /// CLI arguments take precedence over config file settings
    pub fn merge_with_config(&self, mut config: Config) -> Config {
        self.apply_overrides(&mut config);
        config
    }

    /// Convert CLI arguments to Config (when no config file is used)
    pub fn to_config(&self) -> Config {
        // 有意行为差异（勿“修掉”）：纯 CLI 无 --classify 时强制按年月。
        // 带配置文件路径时（merge_with_config）继承文件值，不强制。
        let classification = match self.classify {
            Some(classify) => classify,
            None => ClassificationRule::YearMonth,
        };
        let mut config = Config {
            classification,
            ..Default::default()
        };

        self.apply_overrides(&mut config);
        config
    }
}

/// 是否运行交互模式（无参数或含 `-i/--interactive` 时启用）
pub fn should_run_interactive() -> bool {
    let args: Vec<String> = std::env::args().collect();
    should_run_interactive_for_args(&args)
}

/// 纯函数：根据原始 argv 判定是否进入交互模式
///
/// 在 clap 解析前执行，直接扫描原始 argv：
/// 无参数（仅程序名）→ 交互；带 `-i` / `--interactive` → 交互；
/// 其余带参场景 → CLI。
pub fn should_run_interactive_for_args(args: &[String]) -> bool {
    if args.len() == 1 {
        return true;
    }
    args.iter()
        .skip(1)
        .any(|arg| arg == "-i" || arg == "--interactive")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base_cli() -> Cli {
        Cli {
            config: None,
            input: None,
            output: None,
            mode: None,
            classify: None,
            month_format: None,
            classify_by_type: false,
            operation: None,
            no_deduplicate: false,
            state_file: None,
            threads: None,
            large_file_mb: None,
            dry_run: false,
            unify_filenames: false,
            unmodified_list: None,
            verbose: false,
            json_log: false,
            interactive: false,
        }
    }

    fn file_config() -> Config {
        Config {
            input_dirs: vec![PathBuf::from("D:/config_input")],
            output_dir: PathBuf::from("D:/config_output"),
            processing_mode: ProcessingMode::Supplement,
            classification: ClassificationRule::Year,
            month_format: MonthFormat::Combined,
            classify_by_type: true,
            operation: FileOperation::Move,
            deduplicate: true,
            state_file: Some(PathBuf::from("D:/config_state.json")),
            threads: 4,
            large_file_threshold: 64 * 1024 * 1024,
            dry_run: false,
            verbose: false,
            ..Default::default()
        }
    }

    #[test]
    fn test_to_config_no_classify_forces_year_month() {
        // 纯 CLI 无 --classify 时强制按年月（有意行为差异）
        let cli = base_cli();
        let config = cli.to_config();

        assert_eq!(config.classification, ClassificationRule::YearMonth);
        assert_eq!(config.processing_mode, ProcessingMode::Incremental);
        assert!(config.deduplicate);
    }

    #[test]
    fn test_to_config_classify_preserved_when_provided() {
        let mut cli = base_cli();
        cli.classify = Some(ClassificationRule::Year);
        let config = cli.to_config();

        assert_eq!(config.classification, ClassificationRule::Year);
    }

    #[test]
    fn test_to_config_maps_all_cli_fields() {
        let mut cli = base_cli();
        cli.input = Some(vec![PathBuf::from("D:/in1"), PathBuf::from("D:/in2")]);
        cli.output = Some(PathBuf::from("D:/out"));
        cli.mode = Some(ProcessingMode::Full);
        cli.month_format = Some(MonthFormat::Combined);
        cli.classify_by_type = true;
        cli.operation = Some(FileOperation::Hardlink);
        cli.no_deduplicate = true;
        cli.state_file = Some(PathBuf::from("D:/state.json"));
        cli.threads = Some(8);
        cli.large_file_mb = Some(200);
        cli.dry_run = true;
        cli.verbose = true;

        let config = cli.to_config();

        assert_eq!(
            config.input_dirs,
            vec![PathBuf::from("D:/in1"), PathBuf::from("D:/in2")]
        );
        assert_eq!(config.output_dir, PathBuf::from("D:/out"));
        assert_eq!(config.processing_mode, ProcessingMode::Full);
        assert_eq!(config.month_format, MonthFormat::Combined);
        assert!(config.classify_by_type);
        assert_eq!(config.operation, FileOperation::Hardlink);
        assert!(!config.deduplicate);
        assert_eq!(config.state_file, Some(PathBuf::from("D:/state.json")));
        assert_eq!(config.threads, 8);
        assert_eq!(config.large_file_threshold, 200 * 1024 * 1024);
        assert!(config.dry_run);
        assert!(config.verbose);
    }

    #[test]
    fn test_merge_with_config_file_values_inherited_without_cli_override() {
        // 配置文件路径：无 CLI 覆盖时继承文件值（与 to_config 的强制 YearMonth 相反）
        let cli = base_cli();
        let config = cli.merge_with_config(file_config());

        assert_eq!(config.input_dirs, vec![PathBuf::from("D:/config_input")]);
        assert_eq!(config.output_dir, PathBuf::from("D:/config_output"));
        assert_eq!(config.processing_mode, ProcessingMode::Supplement);
        assert_eq!(config.classification, ClassificationRule::Year);
        assert_eq!(config.month_format, MonthFormat::Combined);
        assert!(config.classify_by_type);
        assert_eq!(config.operation, FileOperation::Move);
        assert_eq!(
            config.state_file,
            Some(PathBuf::from("D:/config_state.json"))
        );
        assert_eq!(config.threads, 4);
        assert_eq!(config.large_file_threshold, 64 * 1024 * 1024);
    }

    #[test]
    fn test_merge_with_config_cli_overrides_file_values() {
        let mut cli = base_cli();
        cli.input = Some(vec![PathBuf::from("D:/cli_input")]);
        cli.output = Some(PathBuf::from("D:/cli_output"));
        cli.mode = Some(ProcessingMode::Full);
        cli.classify = Some(ClassificationRule::None);
        cli.month_format = Some(MonthFormat::Nested);
        cli.operation = Some(FileOperation::Copy);
        cli.no_deduplicate = true;
        cli.state_file = Some(PathBuf::from("D:/cli_state.json"));
        cli.threads = Some(2);
        cli.large_file_mb = Some(10);
        cli.dry_run = true;
        cli.verbose = true;

        let config = cli.merge_with_config(file_config());

        assert_eq!(config.input_dirs, vec![PathBuf::from("D:/cli_input")]);
        assert_eq!(config.output_dir, PathBuf::from("D:/cli_output"));
        assert_eq!(config.processing_mode, ProcessingMode::Full);
        assert_eq!(config.classification, ClassificationRule::None);
        assert_eq!(config.month_format, MonthFormat::Nested);
        assert_eq!(config.operation, FileOperation::Copy);
        assert!(!config.deduplicate);
        assert_eq!(config.state_file, Some(PathBuf::from("D:/cli_state.json")));
        assert_eq!(config.threads, 2);
        assert_eq!(config.large_file_threshold, 10 * 1024 * 1024);
        assert!(config.dry_run);
        assert!(config.verbose);
    }

    #[test]
    fn test_merge_with_config_no_classify_preserves_file_classification() {
        // YearMonth 差异的另一方向：带配置文件时无 --classify 不强制按年月
        let cli = base_cli();
        let config = Config {
            classification: ClassificationRule::None,
            ..Default::default()
        };

        let merged = cli.merge_with_config(config);

        assert_eq!(merged.classification, ClassificationRule::None);
    }

    #[test]
    fn test_apply_overrides_does_not_touch_classification_without_flag() {
        // apply_overrides 本身不处理 YearMonth 特殊默认（那是 to_config 的职责）
        let cli = base_cli();
        let mut config = Config {
            classification: ClassificationRule::Year,
            ..Default::default()
        };

        cli.apply_overrides(&mut config);

        assert_eq!(config.classification, ClassificationRule::Year);
    }

    #[test]
    fn test_unify_filenames_flag_maps_to_config() {
        let mut cli = base_cli();
        cli.unify_filenames = true;
        cli.unmodified_list = Some(PathBuf::from("D:/reports/unmodified.txt"));

        let config = cli.to_config();

        assert!(config.unify_filenames);
        assert_eq!(
            config.unmodified_list_file,
            Some(PathBuf::from("D:/reports/unmodified.txt"))
        );
    }

    #[test]
    fn test_clap_parses_unify_flags() {
        let cli = Cli::try_parse_from([
            "gallery-sorter",
            "--unify-filenames",
            "--unmodified-list",
            "D:/reports/unmodified.txt",
        ])
        .unwrap();

        assert!(cli.unify_filenames);
        assert_eq!(
            cli.unmodified_list,
            Some(PathBuf::from("D:/reports/unmodified.txt"))
        );
    }

    #[test]
    fn test_config_name_uses_file_stem() {
        let mut cli = base_cli();
        cli.config = Some(PathBuf::from("D:/Configs/my_album.toml"));

        assert_eq!(cli.config_name(), Some("my_album".to_string()));
    }

    #[test]
    fn test_should_run_interactive_no_args_returns_true() {
        let args = vec!["gallery-sorter".to_string()];
        assert!(should_run_interactive_for_args(&args));
    }

    #[test]
    fn test_should_run_interactive_with_args_returns_false() {
        let args = vec![
            "gallery-sorter".to_string(),
            "--input".to_string(),
            "D:/photos".to_string(),
        ];
        assert!(!should_run_interactive_for_args(&args));
    }

    #[test]
    fn test_should_run_interactive_with_dash_i_flag() {
        let args = vec!["gallery-sorter".to_string(), "-i".to_string()];
        assert!(should_run_interactive_for_args(&args));
    }

    #[test]
    fn test_should_run_interactive_with_long_flag() {
        let args = vec!["gallery-sorter".to_string(), "--interactive".to_string()];
        assert!(should_run_interactive_for_args(&args));
    }

    #[test]
    fn test_should_run_interactive_flag_with_other_args() {
        let args = vec![
            "gallery-sorter".to_string(),
            "--input".to_string(),
            "D:/photos".to_string(),
            "-i".to_string(),
        ];
        assert!(should_run_interactive_for_args(&args));
    }

    #[test]
    fn test_should_run_interactive_flag_last_position() {
        let args = vec![
            "gallery-sorter".to_string(),
            "--output".to_string(),
            "D:/sorted".to_string(),
            "--interactive".to_string(),
        ];
        assert!(should_run_interactive_for_args(&args));
    }

    #[test]
    fn test_clap_parses_interactive_flag_with_other_args() {
        let cli = Cli::try_parse_from(["gallery-sorter", "--input", "D:/photos", "-i"]).unwrap();

        assert!(cli.interactive);
        assert_eq!(cli.input, Some(vec![PathBuf::from("D:/photos")]));
    }
}
