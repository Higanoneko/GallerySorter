//! CLI 配置加载与校验

use crate::cli::Cli;
use crate::config::Config;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Convenience macro for translation
macro_rules! t {
    ($key:expr) => {
        rust_i18n::t!($key)
    };
    ($key:expr, $($tt:tt)*) => {
        rust_i18n::t!($key, $($tt)*)
    };
}

/// Resolve config path - supports shorthand syntax
pub fn resolve_config_path(exe_dir: &Path, config_path: &Path) -> PathBuf {
    if config_path.exists() {
        return config_path.to_path_buf();
    }

    let with_extension = if config_path.extension().is_none() {
        config_path.with_extension("toml")
    } else {
        config_path.to_path_buf()
    };

    if with_extension.exists() {
        return with_extension;
    }

    let config_dir = exe_dir.join("Config");
    let filename = config_path.file_name().unwrap_or(config_path.as_os_str());

    let mut in_config_dir = config_dir.join(filename);
    if in_config_dir.extension().is_none() {
        in_config_dir = in_config_dir.with_extension("toml");
    }

    if in_config_dir.exists() {
        return in_config_dir;
    }

    config_path.to_path_buf()
}

/// 加载配置并返回解析结果与配置文件路径（如有）
pub fn load_config(cli: &Cli, exe_dir: &Path) -> Result<(Config, Option<PathBuf>)> {
    let (config, config_path) = if let Some(ref config_path) = cli.config {
        let resolved_path = resolve_config_path(exe_dir, config_path);
        let file_config = Config::load_from_file(&resolved_path)?;
        (cli.merge_with_config(file_config), Some(resolved_path))
    } else {
        (cli.to_config(), None)
    };

    if config.input_dirs.is_empty() {
        anyhow::bail!("{}", t!("cli_no_input_dirs_error"));
    }

    Ok((config, config_path))
}

/// Validate configuration before processing
pub fn validate_config(config: &Config) -> Result<()> {
    for input_dir in &config.input_dirs {
        if !input_dir.exists() {
            eprintln!("{} {}", t!("cli_input_dir_not_exist"), input_dir.display());
        }
    }

    for input_dir in &config.input_dirs {
        if config.output_dir.starts_with(input_dir) {
            anyhow::bail!(
                "{} {} {} {}",
                t!("cli_output_inside_input_error"),
                config.output_dir.display(),
                t!("cli_is_inside"),
                input_dir.display()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::fs;
    use tempfile::tempdir;

    fn write_config(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn valid_config_content() -> String {
        toml::to_string_pretty(&crate::config::Config {
            input_dirs: vec![PathBuf::from("D:/photos")],
            output_dir: PathBuf::from("D:/sorted"),
            classification: crate::config::ClassificationRule::Year,
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn test_resolve_config_path_direct_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_config(&path, &valid_config_content());

        let resolved = resolve_config_path(dir.path(), &path);

        assert_eq!(resolved, path);
    }

    #[test]
    fn test_resolve_config_path_adds_toml_extension() {
        let dir = tempdir().unwrap();
        let with_extension = dir.path().join("config.toml");
        write_config(&with_extension, &valid_config_content());

        let resolved = resolve_config_path(dir.path(), &dir.path().join("config"));

        assert_eq!(resolved, with_extension);
    }

    #[test]
    fn test_resolve_config_path_shorthand_in_config_dir() {
        let dir = tempdir().unwrap();
        let config_dir = dir.path().join("Config");
        let in_config_dir = config_dir.join("my_album.toml");
        write_config(&in_config_dir, &valid_config_content());

        let resolved = resolve_config_path(dir.path(), Path::new("my_album"));

        assert_eq!(resolved, in_config_dir);
    }

    #[test]
    fn test_resolve_config_path_missing_returns_input() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing.toml");

        let resolved = resolve_config_path(dir.path(), &missing);

        assert_eq!(resolved, missing);
    }

    #[test]
    fn test_load_config_with_file_merges_and_returns_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_config(&path, &valid_config_content());

        let cli = Cli::try_parse_from([
            "gallery-sorter",
            "--config",
            path.to_str().unwrap(),
            "--output",
            "D:/cli_out",
        ])
        .unwrap();

        let (config, config_path) = load_config(&cli, dir.path()).unwrap();

        assert_eq!(config_path, Some(path));
        assert_eq!(config.input_dirs, vec![PathBuf::from("D:/photos")]);
        assert_eq!(config.output_dir, PathBuf::from("D:/cli_out"));
        assert_eq!(
            config.classification,
            crate::config::ClassificationRule::Year
        );
    }

    #[test]
    fn test_load_config_without_file_returns_defaults_and_none() {
        let dir = tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "gallery-sorter",
            "--input",
            "D:/photos",
            "--output",
            "D:/sorted",
        ])
        .unwrap();

        let (config, config_path) = load_config(&cli, dir.path()).unwrap();

        assert_eq!(config_path, None);
        assert_eq!(config.input_dirs, vec![PathBuf::from("D:/photos")]);
        // 纯 CLI 无 --classify 时强制按年月
        assert_eq!(
            config.classification,
            crate::config::ClassificationRule::YearMonth
        );
    }

    #[test]
    fn test_load_config_empty_input_dirs_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = toml::to_string_pretty(&crate::config::Config::default()).unwrap();
        write_config(&path, &content);

        let cli =
            Cli::try_parse_from(["gallery-sorter", "--config", path.to_str().unwrap()]).unwrap();

        let err = load_config(&cli, dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("input directories") || msg.contains("输入目录"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn test_validate_config_output_inside_input_errors() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("photos");
        fs::create_dir_all(&input).unwrap();

        let config = Config {
            input_dirs: vec![input.clone()],
            output_dir: input.join("sorted"),
            ..Default::default()
        };

        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("sorted") && msg.contains("photos"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn test_validate_config_valid_config_ok() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("photos");
        let output = dir.path().join("sorted");
        fs::create_dir_all(&input).unwrap();

        let config = Config {
            input_dirs: vec![input],
            output_dir: output,
            ..Default::default()
        };

        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_load_from_file_missing_required_field_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        write_config(
            &path,
            r#"
output_dir = "D:/sorted"
processing_mode = "full"
"#,
        );

        let err = Config::load_from_file(&path).unwrap_err();
        assert!(matches!(err, crate::config::ConfigError::ParseError { .. }));
    }

    #[test]
    fn test_load_from_file_wrong_type_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("wrong_type.toml");
        write_config(
            &path,
            r#"
input_dirs = "D:/photos"
output_dir = "D:/sorted"
processing_mode = "full"
"#,
        );

        let err = Config::load_from_file(&path).unwrap_err();
        assert!(matches!(err, crate::config::ConfigError::ParseError { .. }));
    }

    #[test]
    fn test_load_from_file_invalid_enum_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid_enum.toml");
        write_config(
            &path,
            r#"
input_dirs = ["D:/photos"]
output_dir = "D:/sorted"
processing_mode = "banana"
"#,
        );

        let err = Config::load_from_file(&path).unwrap_err();
        assert!(matches!(err, crate::config::ConfigError::ParseError { .. }));
    }
}
