//! 归档目标路径构建
//!
//! 负责根据时间、分类规则与“统一文件名”选项计算目标路径与文件名，
//! 并把未修改文件筛选逻辑集中在此处（AGENTS.md：新增阶段逻辑拆分子文件）。

use super::{FileResult, ProcessingStatus};
use crate::config::{ClassificationRule, Config, FileType, MonthFormat};
use crate::error::{Error, Result};
use crate::rename::{build_unified_filename, is_standard_name};
use crate::time::{ExtractedTime, TimeSource};
use chrono::Datelike;
use std::path::{Path, PathBuf};

/// 计算归档时的目标文件名。
///
/// - 未开启统一文件名：使用源文件名。
/// - 开启且元数据可解析（EXIF / FFprobe）：使用标准前缀统一名
///   （`MVIMG_` / `IMG_` / `VID_` + 时间戳）。
/// - 开启但命中 `preserve_standard_names`：保持源文件名。
/// - 开启但无元数据（文件名 / 文件系统时间回退）：保持源文件名，
///   该文件会进入未修改列表。
pub(super) fn destination_file_name(
    source: &Path,
    time_info: &ExtractedTime,
    config: &Config,
) -> PathBuf {
    let original = source
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(source.as_os_str()));

    if !config.unify_filenames {
        return original;
    }
    if config.preserve_standard_names && is_standard_name(source) {
        return original;
    }

    match time_info.source {
        TimeSource::Exif | TimeSource::VideoMetadata => {
            build_unified_filename(source, time_info, config)
                .file_name()
                .map(PathBuf::from)
                .unwrap_or(original)
        }
        TimeSource::Filename | TimeSource::FileSystem => original,
    }
}

/// 判断结果是否应写入“未修改文件列表”（仅统一文件名模式生效）。
///
/// 与旧 rename 模式语义一致：无元数据未改名 + 处理失败计入；
/// 被 `preserve_standard_names` 保留的标准名称不计入。
pub(super) fn should_list_as_unmodified(result: &FileResult, config: &Config) -> bool {
    if !config.unify_filenames {
        return false;
    }
    if result.status == ProcessingStatus::Failed {
        return true;
    }
    if result.status != ProcessingStatus::Success && result.status != ProcessingStatus::DryRun {
        return false;
    }
    if config.preserve_standard_names && is_standard_name(&result.source) {
        return false;
    }
    matches!(
        result.time_info.as_ref().map(|t| t.source),
        Some(TimeSource::Filename | TimeSource::FileSystem)
    )
}

/// Build the base destination path based on classification rules (without conflict resolution)
pub(super) fn build_base_destination_path(
    source: &Path,
    time_info: &ExtractedTime,
    config: &Config,
) -> Result<PathBuf> {
    let filename = if config.unify_filenames {
        destination_file_name(source, time_info, config)
    } else {
        source
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(|| Error::Config("Invalid source filename".into()))?
    };

    let mut dest = config.output_dir.clone();

    // Time-based classification
    match config.classification {
        ClassificationRule::None => {
            // Files go directly to output directory
        }
        ClassificationRule::Year => {
            dest.push(format!("{}", time_info.timestamp.year()));
        }
        ClassificationRule::YearMonth => match config.month_format {
            MonthFormat::Nested => {
                dest.push(format!("{}", time_info.timestamp.year()));
                dest.push(format!("{:02}", time_info.timestamp.month()));
            }
            MonthFormat::Combined => {
                dest.push(format!(
                    "{}-{:02}",
                    time_info.timestamp.year(),
                    time_info.timestamp.month()
                ));
            }
        },
    }

    // File type classification (after time classification)
    if config.classify_by_type
        && let Some(ext) = source.extension().and_then(|e| e.to_str())
        && let Some(file_type) = config.get_file_type(ext)
    {
        match file_type {
            FileType::Photos => {
                dest.push(file_type.folder_name());
            }
            FileType::Raw => {
                // RAW files are nested under Photos/Raw
                dest.push(FileType::Photos.folder_name());
                dest.push(file_type.folder_name());
            }
            FileType::Videos => {
                dest.push(file_type.folder_name());
            }
        }
    }

    dest.push(filename);
    Ok(dest)
}
