//! 文件名统一化（Rename）模块
//!
//! 基于 EXIF（图片）/ FFprobe（视频）元数据把文件名统一为
//! `YYYYMMDD_HHMMSS.ext`。没有可解析元数据的文件保持原名，
//! 可通过 [`write_unmodified_list`] 把这类文件输出到列表文件。

use crate::config::Config;
use crate::error::Result;
use crate::process::{collect_media_files, resolve_filename_conflict_with};
use crate::time::extract_metadata_time;
use chrono::NaiveDateTime;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tracing::{Level, debug, error, info, span};

/// 单个文件的统一化结果
#[derive(Debug, Clone)]
pub struct RenameResult {
    /// 源文件路径
    pub source: PathBuf,
    /// 统一化后的路径（已重命名 / 试运行时有值）
    pub destination: Option<PathBuf>,
    /// 处理状态
    pub status: RenameStatus,
    /// 错误信息（失败时）
    pub error: Option<String>,
}

/// 文件统一化状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameStatus {
    /// 已成功重命名
    Renamed,
    /// 试运行：本应重命名但未实际修改
    DryRun,
    /// 文件名已是统一格式，无需修改
    AlreadyUnified,
    /// 无 EXIF / FFprobe 元数据，保持原名
    NoMetadata,
    /// 重命名失败（保持原名）
    Failed,
    /// 取消时未处理
    Cancelled,
}

/// 文件统一化统计
#[derive(Debug, Default)]
pub struct RenameStats {
    pub total_files: AtomicUsize,
    /// 已处理（重命名 / 已统一 / 无元数据 / 失败），用于进度显示
    pub handled: AtomicUsize,
    pub renamed: AtomicUsize,
    pub already_unified: AtomicUsize,
    pub no_metadata: AtomicUsize,
    pub failed: AtomicUsize,
}

impl Clone for RenameStats {
    fn clone(&self) -> Self {
        Self {
            total_files: AtomicUsize::new(self.total_files.load(Ordering::Relaxed)),
            handled: AtomicUsize::new(self.handled.load(Ordering::Relaxed)),
            renamed: AtomicUsize::new(self.renamed.load(Ordering::Relaxed)),
            already_unified: AtomicUsize::new(self.already_unified.load(Ordering::Relaxed)),
            no_metadata: AtomicUsize::new(self.no_metadata.load(Ordering::Relaxed)),
            failed: AtomicUsize::new(self.failed.load(Ordering::Relaxed)),
        }
    }
}

impl RenameStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn summary(&self) -> String {
        format!(
            "Total: {}, Renamed: {}, Already unified: {}, No metadata: {}, Failed: {}",
            self.total_files.load(Ordering::Relaxed),
            self.renamed.load(Ordering::Relaxed),
            self.already_unified.load(Ordering::Relaxed),
            self.no_metadata.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed)
        )
    }
}

/// 构建统一文件名：`YYYYMMDD_HHMMSS` + 原扩展名（保留扩展名大小写）
pub fn build_unified_filename(source: &Path, timestamp: &NaiveDateTime) -> PathBuf {
    let stem = timestamp.format("%Y%m%d_%H%M%S").to_string();
    let file_name = match source.extension() {
        Some(ext) => format!("{}.{}", stem, ext.to_string_lossy()),
        None => stem,
    };
    source.with_file_name(file_name)
}

/// 判断文件当前是否已是统一文件名（与目标完全一致）
pub fn is_already_unified(source: &Path, timestamp: &NaiveDateTime) -> bool {
    build_unified_filename(source, timestamp).as_path() == source
}

/// 把未修改（无元数据或重命名失败）的文件路径写入列表文件（原子写：tmp + rename）。
///
/// 返回写入的文件数量；没有未修改文件时不创建文件。
pub fn write_unmodified_list(path: &Path, results: &[RenameResult]) -> Result<usize> {
    let entries: Vec<String> = results
        .iter()
        .filter(|r| matches!(r.status, RenameStatus::NoMetadata | RenameStatus::Failed))
        .map(|r| r.source.display().to_string())
        .collect();

    if entries.is_empty() {
        return Ok(0);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = PathBuf::from(format!("{}.tmp", path.display()));
    let mut content = String::new();
    for entry in &entries {
        content.push_str(entry);
        content.push('\n');
    }

    fs::write(&temp_path, &content)?;
    fs::rename(&temp_path, path)?;

    Ok(entries.len())
}

/// 文件名统一化器
///
/// 流水线：扫描（复用 `process::collect_media_files`）→ 并行提取
/// EXIF/FFprobe 元数据 → 顺序重命名（冲突加 `_1`/`_2` 后缀）。
pub struct Renamer {
    config: Config,
    stats: Arc<RenameStats>,
    cancel: Arc<AtomicBool>,
}

/// 单个文件在并行提取阶段的规划动作（不修改磁盘）
enum PlannedAction {
    Rename(PathBuf),
    AlreadyUnified,
    NoMetadata,
    Cancelled,
}

impl Renamer {
    /// 创建统一化器
    pub fn new(config: Config) -> Result<Self> {
        Self::new_with_cancel(config, Arc::new(AtomicBool::new(false)))
    }

    /// 创建统一化器（带取消标志）
    pub fn new_with_cancel(config: Config, cancel: Arc<AtomicBool>) -> Result<Self> {
        if config.threads > 0 {
            rayon::ThreadPoolBuilder::new()
                .num_threads(config.threads)
                .build_global()
                .ok(); // Ignore if already initialized
        }

        Ok(Self {
            config,
            stats: Arc::new(RenameStats::new()),
            cancel,
        })
    }

    /// 是否已请求取消
    pub fn was_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 获取待处理文件总数（供进度显示）
    pub fn total_files_count(&self) -> Result<usize> {
        Ok(collect_media_files(&self.config)?.len())
    }

    /// 获取统计引用
    pub fn stats(&self) -> &RenameStats {
        &self.stats
    }

    /// 获取统计的 Arc 克隆（供进度线程共享）
    pub fn stats_arc(&self) -> Arc<RenameStats> {
        self.stats.clone()
    }

    /// 运行文件名统一化流水线
    pub fn run(&mut self) -> Result<Vec<RenameResult>> {
        let _span = span!(Level::INFO, "renamer_run").entered();

        info!("Scanning input directories for filename unification...");
        let files = collect_media_files(&self.config)?;
        info!(count = files.len(), "Found media files");

        if files.is_empty() {
            info!("No files to unify");
            return Ok(Vec::new());
        }

        self.stats.total_files.store(files.len(), Ordering::Relaxed);
        let config = Arc::new(self.config.clone());
        let cancel = self.cancel.clone();

        // Phase 1: 并行提取元数据并规划目标文件名（不修改磁盘）
        info!("Extracting EXIF / FFprobe metadata...");
        let planned: Vec<(PathBuf, PlannedAction)> = files
            .par_iter()
            .map(|path| {
                if cancel.load(Ordering::Relaxed) {
                    return (path.clone(), PlannedAction::Cancelled);
                }

                match extract_metadata_time(path, &config) {
                    Ok(time) => {
                        let target = build_unified_filename(path, &time.timestamp);
                        if target.as_path() == path {
                            (path.clone(), PlannedAction::AlreadyUnified)
                        } else {
                            (path.clone(), PlannedAction::Rename(target))
                        }
                    }
                    Err(_) => (path.clone(), PlannedAction::NoMetadata),
                }
            })
            .collect();

        // Phase 2: 顺序重命名（目标冲突加 _N 后缀；取消后已完成的保留）
        info!("Renaming files...");
        let mut results = Vec::with_capacity(planned.len());
        let mut occupied: HashSet<PathBuf> = HashSet::new();

        for (source, action) in planned {
            if cancel.load(Ordering::Relaxed) {
                debug!(?source, "File not processed due to cancellation");
                results.push(RenameResult {
                    source,
                    destination: None,
                    status: RenameStatus::Cancelled,
                    error: None,
                });
                continue;
            }

            match action {
                PlannedAction::Rename(target) => {
                    match resolve_filename_conflict_with(target, &occupied) {
                        Ok(final_target) => {
                            if self.config.dry_run {
                                debug!(?source, ?final_target, "Would rename file");
                                self.stats.handled.fetch_add(1, Ordering::Relaxed);
                                self.stats.renamed.fetch_add(1, Ordering::Relaxed);
                                occupied.insert(final_target.clone());
                                results.push(RenameResult {
                                    source,
                                    destination: Some(final_target),
                                    status: RenameStatus::DryRun,
                                    error: None,
                                });
                            } else {
                                match fs::rename(&source, &final_target) {
                                    Ok(()) => {
                                        info!(
                                            source = ?source,
                                            destination = ?final_target,
                                            "Renamed file"
                                        );
                                        self.stats.handled.fetch_add(1, Ordering::Relaxed);
                                        self.stats.renamed.fetch_add(1, Ordering::Relaxed);
                                        occupied.insert(final_target.clone());
                                        results.push(RenameResult {
                                            source,
                                            destination: Some(final_target),
                                            status: RenameStatus::Renamed,
                                            error: None,
                                        });
                                    }
                                    Err(e) => {
                                        error!(
                                            ?source,
                                            ?final_target,
                                            error = %e,
                                            "Failed to rename file"
                                        );
                                        self.stats.handled.fetch_add(1, Ordering::Relaxed);
                                        self.stats.failed.fetch_add(1, Ordering::Relaxed);
                                        results.push(RenameResult {
                                            source,
                                            destination: Some(final_target),
                                            status: RenameStatus::Failed,
                                            error: Some(e.to_string()),
                                        });
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(?source, error = %e, "Failed to resolve rename conflict");
                            self.stats.handled.fetch_add(1, Ordering::Relaxed);
                            self.stats.failed.fetch_add(1, Ordering::Relaxed);
                            results.push(RenameResult {
                                source,
                                destination: None,
                                status: RenameStatus::Failed,
                                error: Some(e.to_string()),
                            });
                        }
                    }
                }
                PlannedAction::AlreadyUnified => {
                    debug!(?source, "Filename already unified");
                    self.stats.handled.fetch_add(1, Ordering::Relaxed);
                    self.stats.already_unified.fetch_add(1, Ordering::Relaxed);
                    results.push(RenameResult {
                        source,
                        destination: None,
                        status: RenameStatus::AlreadyUnified,
                        error: None,
                    });
                }
                PlannedAction::NoMetadata => {
                    debug!(?source, "No EXIF/FFprobe metadata, leaving unchanged");
                    self.stats.handled.fetch_add(1, Ordering::Relaxed);
                    self.stats.no_metadata.fetch_add(1, Ordering::Relaxed);
                    results.push(RenameResult {
                        source,
                        destination: None,
                        status: RenameStatus::NoMetadata,
                        error: None,
                    });
                }
                PlannedAction::Cancelled => {
                    results.push(RenameResult {
                        source,
                        destination: None,
                        status: RenameStatus::Cancelled,
                        error: None,
                    });
                }
            }
        }

        info!("{}", self.stats.summary());
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use tempfile::tempdir;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// 构造带 DateTimeOriginal 的最小 JPEG（APP1 + EXIF TIFF + EOI）
    fn exif_jpeg(datetime: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(b"II\x2A\x00\x08\x00\x00\x00");
        // IFD0（offset 8）：1 个 ExifIFDPointer 条目 → Exif 子 IFD 位于 0x1A
        payload.extend_from_slice(&[0x01, 0x00]);
        payload.extend_from_slice(&[
            0x69, 0x87, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1A, 0x00, 0x00, 0x00,
        ]);
        payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // next IFD = 0
        // Exif 子 IFD（offset 0x1A）：1 个 DateTimeOriginal 条目
        payload.extend_from_slice(&[0x01, 0x00]);
        payload.extend_from_slice(&[
            0x03, 0x90, 0x02, 0x00, 0x14, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00,
        ]);
        payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // next IFD = 0
        payload.extend_from_slice(datetime.as_bytes());
        payload.push(0);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFF, 0xD8]); // SOI
        bytes.extend_from_slice(&[0xFF, 0xE1]); // APP1
        bytes.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&[0xFF, 0xD9]); // EOI
        bytes
    }

    fn rename_config(input: &Path) -> Config {
        Config {
            input_dirs: vec![input.to_path_buf()],
            output_dir: input.to_path_buf(),
            threads: 1,
            ..Default::default()
        }
    }

    #[test]
    fn test_build_unified_filename() {
        let dir = Path::new("/photos");
        let source = dir.join("IMG_20240115_143000.jpg");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"));
        assert_eq!(target, dir.join("20240115_143000.jpg"));

        // 扩展名大小写保留
        let source = dir.join("IMG_20240115_143000.JPG");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"));
        assert_eq!(target, dir.join("20240115_143000.JPG"));

        // 无扩展名
        let source = dir.join("IMG_20240115_143000");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"));
        assert_eq!(target, dir.join("20240115_143000"));
    }

    #[test]
    fn test_is_already_unified() {
        let dir = Path::new("/photos");
        let timestamp = dt("2024-01-15 14:30:00");

        assert!(is_already_unified(
            &dir.join("20240115_143000.jpg"),
            &timestamp
        ));
        assert!(!is_already_unified(
            &dir.join("IMG_20240115_143000.jpg"),
            &timestamp
        ));
    }

    #[test]
    fn test_write_unmodified_list() {
        let dir = tempdir().unwrap();
        let list_path = dir.path().join("unmodified_files.txt");

        let results = vec![
            RenameResult {
                source: PathBuf::from("D:/photos/no_meta.jpg"),
                destination: None,
                status: RenameStatus::NoMetadata,
                error: None,
            },
            RenameResult {
                source: PathBuf::from("D:/photos/broken.jpg"),
                destination: Some(PathBuf::from("D:/photos/20240115_143000.jpg")),
                status: RenameStatus::Failed,
                error: Some("io error".into()),
            },
            RenameResult {
                source: PathBuf::from("D:/photos/ok.jpg"),
                destination: Some(PathBuf::from("D:/photos/20240115_143000_1.jpg")),
                status: RenameStatus::Renamed,
                error: None,
            },
        ];

        let count = write_unmodified_list(&list_path, &results).unwrap();
        assert_eq!(count, 2);
        let content = fs::read_to_string(&list_path).unwrap();
        assert!(content.contains("D:/photos/no_meta.jpg\n"));
        assert!(content.contains("D:/photos/broken.jpg\n"));
        assert!(!content.contains("ok.jpg"));
    }

    #[test]
    fn test_write_unmodified_list_empty_creates_no_file() {
        let dir = tempdir().unwrap();
        let list_path = dir.path().join("unmodified_files.txt");

        let count = write_unmodified_list(&list_path, &[]).unwrap();
        assert_eq!(count, 0);
        assert!(!list_path.exists());
    }

    #[test]
    fn test_renamer_renames_by_metadata_and_keeps_metadata_less_files() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fs::create_dir_all(&input).unwrap();

        // 有 EXIF：重命名为统一格式
        fs::write(
            input.join("IMG_20240115_143000.jpg"),
            exif_jpeg("2024:01:15 14:30:00"),
        )
        .unwrap();
        // 同一时间戳的第二张：冲突加 _1 后缀
        fs::write(
            input.join("IMG_20240115_143000_2.jpg"),
            exif_jpeg("2024:01:15 14:30:00"),
        )
        .unwrap();
        // 已经是统一格式：不修改
        fs::write(
            input.join("20240115_153000.jpg"),
            exif_jpeg("2024:01:15 15:30:00"),
        )
        .unwrap();
        // 无 EXIF：保持原名
        fs::write(input.join("no_meta.jpg"), b"not a jpeg").unwrap();

        let config = Config {
            output_dir: output,
            ..rename_config(&input)
        };
        let mut renamer = Renamer::new(config).unwrap();
        let results = renamer.run().unwrap();

        let statuses: Vec<RenameStatus> = results.iter().map(|r| r.status).collect();
        assert!(statuses.contains(&RenameStatus::Renamed));
        assert!(statuses.contains(&RenameStatus::AlreadyUnified));
        assert!(statuses.contains(&RenameStatus::NoMetadata));

        // 已重命名
        assert!(input.join("20240115_143000.jpg").exists());
        assert!(input.join("20240115_143000_1.jpg").exists());
        // 已是统一格式的原名仍在
        assert!(input.join("20240115_153000.jpg").exists());
        // 无元数据文件未动
        assert!(input.join("no_meta.jpg").exists());
        assert!(!input.join("IMG_20240115_143000.jpg").exists());
        assert!(!input.join("IMG_20240115_143000_2.jpg").exists());

        let stats = renamer.stats();
        assert_eq!(stats.handled.load(Ordering::Relaxed), 4);
        assert_eq!(stats.renamed.load(Ordering::Relaxed), 2);
        assert_eq!(stats.already_unified.load(Ordering::Relaxed), 1);
        assert_eq!(stats.no_metadata.load(Ordering::Relaxed), 1);
        assert_eq!(stats.failed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_renamer_dry_run_does_not_modify_files() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        fs::create_dir_all(&input).unwrap();

        fs::write(
            input.join("IMG_20240115_143000.jpg"),
            exif_jpeg("2024:01:15 14:30:00"),
        )
        .unwrap();

        let config = Config {
            dry_run: true,
            ..rename_config(&input)
        };
        let mut renamer = Renamer::new(config).unwrap();
        let results = renamer.run().unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, RenameStatus::DryRun);
        assert!(input.join("IMG_20240115_143000.jpg").exists());
        assert!(!input.join("20240115_143000.jpg").exists());
    }

    #[test]
    fn test_renamer_unmodified_list_writer_includes_no_metadata_and_failed() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("no_meta.jpg"), b"not a jpeg").unwrap();

        let config = rename_config(&input);
        let mut renamer = Renamer::new(config).unwrap();
        let results = renamer.run().unwrap();

        let list_path = dir.path().join("unmodified_files.txt");
        let count = write_unmodified_list(&list_path, &results).unwrap();
        assert_eq!(count, 1);
        let content = fs::read_to_string(&list_path).unwrap();
        assert!(content.contains("no_meta.jpg"));
    }
}
