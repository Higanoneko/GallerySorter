//! 摘要状态

use crate::process::{FileResult, ProcessingStats};
use std::path::PathBuf;

/// 结果摘要状态
#[derive(Debug)]
pub struct SummaryState {
    /// 处理统计
    pub stats: ProcessingStats,
    /// 处理结果
    pub results: Vec<FileResult>,
    /// 是否试运行
    pub dry_run: bool,
    /// 日志路径
    pub log_path: Option<PathBuf>,
    /// 是否被用户取消（ADR-0003：保存到中断点，不回滚）
    pub cancelled: bool,
}

impl SummaryState {
    /// 创建摘要状态
    pub fn new(
        stats: ProcessingStats,
        results: Vec<FileResult>,
        dry_run: bool,
        log_path: Option<PathBuf>,
    ) -> Self {
        Self::new_with_status(stats, results, dry_run, log_path, false)
    }

    /// 创建摘要状态（带取消标志）
    pub fn new_with_status(
        stats: ProcessingStats,
        results: Vec<FileResult>,
        dry_run: bool,
        log_path: Option<PathBuf>,
        cancelled: bool,
    ) -> Self {
        Self {
            stats,
            results,
            dry_run,
            log_path,
            cancelled,
        }
    }
}
