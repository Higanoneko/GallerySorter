//! 文件名统一化（Rename）模块
//!
//! 基于 EXIF（图片）/ FFprobe（视频）元数据把文件名统一为
//! `MVIMG_/IMG_/VID_ + YYYYMMDD_HHMMSS.ext` 的标准相机命名。
//! CLI/TUI 归档流水线在构建目标路径时直接使用这里的命名规则；
//! [`Renamer`] 保留为库层面的“原地统一”工具，供不需要归档的调用方使用。
//! 没有可解析元数据的文件保持原名，可通过 [`write_unmodified_list`] /
//! [`write_unmodified_paths`] 把这类文件输出到列表文件。

use crate::config::Config;
use crate::error::Result;
use crate::process::{collect_media_files, resolve_filename_conflict_with};
use crate::time::extract_metadata_time;
use chrono::NaiveDateTime;
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tracing::{Level, debug, error, info, span};

/// 标准相机命名模式：`<MVIMG|IMG|VID>_<YYYYMMDD>_<HHMMSS>`，其后任意内容均视为标准名称
static STANDARD_NAME_PATTERN: OnceLock<std::result::Result<Regex, regex::Error>> = OnceLock::new();

/// 判断文件名是否为标准类型名称（不区分大小写）。
///
/// 只要文件名以 `<标准开头>_<年月日>_<时分秒>` 开头，无论后面接什么
/// （如 `_HDR`、`_1`、`-edited`）都视为标准名称。
/// 注意：此处的保留集合（MVIMG/IMG/VID）按需求刻意比
/// `time/filename.rs` 中解析文件名时间的相机前缀集合更窄，
/// 两者语义不同，不要合并。
pub fn is_standard_name(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    // 模式来自编译期常量；初始化失败（如正则语法兼容性问题）时保守回退为“非标准”，
    // 避免库代码 panic（AGENTS.md：库代码不得 unwrap/expect）
    match STANDARD_NAME_PATTERN.get_or_init(|| Regex::new(r"(?i)^(?:MVIMG|IMG|VID)_\d{8}_\d{6}")) {
        Ok(pattern) => pattern.is_match(stem),
        Err(e) => {
            debug!(error = %e, "Failed to initialize standard-name pattern");
            false
        }
    }
}

/// 判断文件是否为动态照片（Motion Photo）。
///
/// 优先依据文件名 `MVIMG_` 前缀（Samsung 等相机标准命名）；
/// 对 JPEG 文件再扫描 APP1/XMP 段中的常见 Motion Photo 标记
/// （`MicroVideoOffset` / `MicroVideoVersion` / `MotionPhoto` 等），
/// 覆盖 Google Pixel 等不叫 `MVIMG_` 的动态照片。
pub fn is_dynamic_photo(path: &Path) -> bool {
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && let Some(head) = stem.get(..6)
        && head.eq_ignore_ascii_case("mvimg_")
    {
        return true;
    }

    jpeg_has_motion_photo_marker(path)
}

/// 计算文件统一化时的标准前缀：
/// 视频 → `VID`；动态照片 → `MVIMG`；照片与 RAW → `IMG`。
pub fn standard_name_prefix(source: &Path, config: &Config) -> &'static str {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("");
    if config.is_video(ext) {
        "VID"
    } else if is_dynamic_photo(source) {
        "MVIMG"
    } else {
        "IMG"
    }
}

/// JPEG APP1/XMP 段中常见的 Motion Photo 标记（小写关键字）
const MOTION_PHOTO_MARKERS: &[&str] = &[
    "microvideooffset",
    "microvideoversion",
    "microvideopresentationtimestampus",
    "motionphoto",
];

/// 扫描 JPEG 的 APP1/XMP 段，检查是否包含动态照片标记。
///
/// 仅扫描 `SOS`（图像扫描数据）之前的元数据段，避免误读压缩后的像素数据。
fn jpeg_has_motion_photo_marker(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    // JPEG 必须以 SOI 开头
    let mut soi = [0u8; 2];
    if reader.read_exact(&mut soi).is_err() || soi != [0xFF, 0xD8] {
        return false;
    }

    loop {
        // 丢弃非 0xFF 字节，找到下一个标记前缀
        let mut discarded = Vec::new();
        if reader.read_until(0xFF, &mut discarded).is_err() {
            return false;
        }

        // 读取标记码（跳过连续的 0xFF 填充字节）
        let code = loop {
            let mut byte = [0u8; 1];
            if reader.read_exact(&mut byte).is_err() {
                return false;
            }
            if byte[0] != 0xFF {
                break byte[0];
            }
        };

        // 独立标记：无长度字段
        match code {
            0x00 | 0x01 | 0xD0..=0xD7 => continue,
            // SOI 出现在文件中间属于异常；EOI / SOS 表示元数据段结束
            0xD8..=0xDA => return false,
            _ => {}
        }

        let mut len_buf = [0u8; 2];
        if reader.read_exact(&mut len_buf).is_err() {
            return false;
        }
        let seg_len = u16::from_be_bytes(len_buf) as usize;
        if seg_len < 2 {
            return false;
        }

        let mut segment = vec![0u8; seg_len - 2];
        if reader.read_exact(&mut segment).is_err() {
            return false;
        }

        // APP1 + XMP 标识
        const XMP_ID: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
        if code == 0xE1
            && let Some(content) = segment.strip_prefix(XMP_ID)
            && has_motion_photo_marker(content)
        {
            return true;
        }
    }
}

/// 在 XMP 内容中查找 Motion Photo 标记（大小写不敏感）
fn has_motion_photo_marker(content: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(content).to_ascii_lowercase();
    MOTION_PHOTO_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

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
    /// 标准类型名称（MVIMG_/IMG_/VID_ 开头），按选项跳过处理
    Preserved,
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
    pub preserved: AtomicUsize,
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
            preserved: AtomicUsize::new(self.preserved.load(Ordering::Relaxed)),
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
            "Total: {}, Renamed: {}, Preserved: {}, Already unified: {}, No metadata: {}, Failed: {}",
            self.total_files.load(Ordering::Relaxed),
            self.renamed.load(Ordering::Relaxed),
            self.preserved.load(Ordering::Relaxed),
            self.already_unified.load(Ordering::Relaxed),
            self.no_metadata.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed)
        )
    }
}

/// 构建标准统一文件名：`<MVIMG|IMG|VID>_YYYYMMDD_HHMMSS` + 原扩展名（保留扩展名大小写）
pub fn build_unified_filename(
    source: &Path,
    timestamp: &NaiveDateTime,
    config: &Config,
) -> PathBuf {
    let prefix = standard_name_prefix(source, config);
    let stem = format!("{}_{}", prefix, timestamp.format("%Y%m%d_%H%M%S"));
    let file_name = match source.extension() {
        Some(ext) => format!("{}.{}", stem, ext.to_string_lossy()),
        None => stem,
    };
    source.with_file_name(file_name)
}

/// 判断文件当前是否已是标准统一文件名（与目标完全一致）
pub fn is_already_unified(source: &Path, timestamp: &NaiveDateTime, config: &Config) -> bool {
    build_unified_filename(source, timestamp, config).as_path() == source
}

/// 把未修改（无元数据或重命名失败）的文件路径写入列表文件（原子写：tmp + rename）。
///
/// 返回写入的文件数量；没有未修改文件时不创建文件。
pub fn write_unmodified_list(path: &Path, results: &[RenameResult]) -> Result<usize> {
    let entries: Vec<PathBuf> = results
        .iter()
        .filter(|r| matches!(r.status, RenameStatus::NoMetadata | RenameStatus::Failed))
        .map(|r| r.source.clone())
        .collect();

    write_unmodified_paths(path, &entries)
}

/// 把未修改文件的路径写入列表文件（原子写：tmp + rename）。
///
/// 供归档流水线（Processor）与文件名统一化器共用；
/// 返回写入的文件数量；没有未修改文件时不创建文件。
pub fn write_unmodified_paths(path: &Path, entries: &[PathBuf]) -> Result<usize> {
    if entries.is_empty() {
        return Ok(0);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = PathBuf::from(format!("{}.tmp", path.display()));
    let mut content = String::new();
    for entry in entries {
        content.push_str(&entry.display().to_string());
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
    Preserved,
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

                if config.preserve_standard_names && is_standard_name(path) {
                    return (path.clone(), PlannedAction::Preserved);
                }

                match extract_metadata_time(path, &config) {
                    Ok(time) => {
                        let target = build_unified_filename(path, &time.timestamp, &config);
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
                PlannedAction::Preserved => {
                    debug!(?source, "Standard camera name preserved");
                    self.stats.handled.fetch_add(1, Ordering::Relaxed);
                    self.stats.preserved.fetch_add(1, Ordering::Relaxed);
                    results.push(RenameResult {
                        source,
                        destination: None,
                        status: RenameStatus::Preserved,
                        error: None,
                    });
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

    /// 构造带 XMP Motion Photo 标记的最小 JPEG（APP1 + XMP + EOI）
    fn xmp_motion_jpeg() -> Vec<u8> {
        let xmp_payload =
            b"http://ns.adobe.com/xap/1.0/\0<rdf><MicroVideoOffset>1234</MicroVideoOffset></rdf>";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFF, 0xD8]); // SOI
        bytes.extend_from_slice(&[0xFF, 0xE1]); // APP1
        bytes.extend_from_slice(&((xmp_payload.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(xmp_payload);
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
        let config = Config::default();

        // 照片 → IMG 前缀
        let source = dir.join("DSC_0001.jpg");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"), &config);
        assert_eq!(target, dir.join("IMG_20240115_143000.jpg"));

        // 视频 → VID 前缀
        let source = dir.join("DSC_0001.mp4");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"), &config);
        assert_eq!(target, dir.join("VID_20240115_143000.mp4"));

        // RAW → IMG 前缀
        let source = dir.join("DSC_0001.arw");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"), &config);
        assert_eq!(target, dir.join("IMG_20240115_143000.arw"));

        // 动态照片文件名（MVIMG_ 开头）→ MVIMG 前缀
        let source = dir.join("MVIMG_20240115_143000_HDR.jpg");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"), &config);
        assert_eq!(target, dir.join("MVIMG_20240115_143000.jpg"));

        // 扩展名大小写保留
        let source = dir.join("IMG_20240115_143000.JPG");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"), &config);
        assert_eq!(target, dir.join("IMG_20240115_143000.JPG"));

        // 无扩展名
        let source = dir.join("IMG_20240115_143000");
        let target = build_unified_filename(&source, &dt("2024-01-15 14:30:00"), &config);
        assert_eq!(target, dir.join("IMG_20240115_143000"));
    }

    #[test]
    fn test_is_already_unified() {
        let dir = Path::new("/photos");
        let timestamp = dt("2024-01-15 14:30:00");
        let config = Config::default();

        assert!(is_already_unified(
            &dir.join("IMG_20240115_143000.jpg"),
            &timestamp,
            &config
        ));
        assert!(!is_already_unified(
            &dir.join("20240115_143000.jpg"),
            &timestamp,
            &config
        ));
    }

    #[test]
    fn test_standard_name_prefix() {
        let dir = Path::new("/photos");
        let config = Config::default();

        assert_eq!(standard_name_prefix(&dir.join("photo.jpg"), &config), "IMG");
        assert_eq!(standard_name_prefix(&dir.join("photo.arw"), &config), "IMG");
        assert_eq!(standard_name_prefix(&dir.join("video.mp4"), &config), "VID");
        // MVIMG_ 文件名 → 动态照片
        assert_eq!(
            standard_name_prefix(&dir.join("MVIMG_20240115_143000.jpg"), &config),
            "MVIMG"
        );
    }

    #[test]
    fn test_is_dynamic_photo_detects_filename_and_xmp_marker() {
        let dir = tempdir().unwrap();

        // MVIMG_ 文件名
        let named = dir.path().join("MVIMG_20240115_143000_HDR.jpg");
        fs::write(&named, b"irrelevant").unwrap();
        assert!(is_dynamic_photo(&named));

        // XMP MicroVideoOffset 标记（非 MVIMG_ 文件名）
        let xmp = dir.path().join("PXL_20240115_143000.jpg");
        fs::write(&xmp, xmp_motion_jpeg()).unwrap();
        assert!(is_dynamic_photo(&xmp));

        // 普通 JPEG / 无标记文件
        let plain = dir.path().join("DSC_0001.jpg");
        fs::write(&plain, b"not a jpeg").unwrap();
        assert!(!is_dynamic_photo(&plain));
        assert!(!is_dynamic_photo(&dir.path().join("missing.jpg")));
    }

    #[test]
    fn test_is_standard_name() {
        let dir = Path::new("/photos");

        // 标准开头 + 年月日 + 时分秒
        assert!(is_standard_name(&dir.join("MVIMG_20240115_143000.jpg")));
        assert!(is_standard_name(&dir.join("IMG_20240115_143000.jpg")));
        assert!(is_standard_name(&dir.join("VID_20240115_143000.mp4")));

        // 无论时间部分后面接什么都视为标准名称
        assert!(is_standard_name(&dir.join("IMG_20240115_143000_HDR.jpg")));
        assert!(is_standard_name(&dir.join("IMG_20240115_143000_1.jpg")));
        assert!(is_standard_name(
            &dir.join("MVIMG_20240115_143000-edit.jpg")
        ));

        // 不区分大小写
        assert!(is_standard_name(&dir.join("img_20240115_143000.jpg")));

        // 非标准名称
        assert!(!is_standard_name(&dir.join("20240115_143000.jpg")));
        assert!(!is_standard_name(&dir.join("IMG_20240115.jpg")));
        assert!(!is_standard_name(&dir.join("IMG_2024015_143000.jpg")));
        assert!(!is_standard_name(&dir.join("IMG_20240115_14300.jpg")));
        assert!(!is_standard_name(&dir.join("MYIMG_20240115_143000.jpg")));
        assert!(!is_standard_name(&dir.join("IMGX_20240115_143000.jpg")));
        assert!(!is_standard_name(&dir.join("VIDEO_20240115_143000.mp4")));
        assert!(!is_standard_name(&dir.join("photo.jpg")));
    }

    #[test]
    fn test_renamer_preserves_standard_names_when_enabled() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        fs::create_dir_all(&input).unwrap();

        // 标准名称：即使有 EXIF 也保持原名
        fs::write(
            input.join("IMG_20240115_143000.jpg"),
            exif_jpeg("2024:01:15 14:30:00"),
        )
        .unwrap();
        fs::write(
            input.join("MVIMG_20240115_143000_HDR.jpg"),
            exif_jpeg("2024:01:15 14:30:00"),
        )
        .unwrap();
        // 非标准名称：仍按元数据重命名
        fs::write(input.join("DSC_0001.jpg"), exif_jpeg("2024:01:15 15:30:00")).unwrap();
        // 无元数据：保持原名并进入未修改列表
        fs::write(input.join("no_meta.jpg"), b"not a jpeg").unwrap();

        let config = Config {
            preserve_standard_names: true,
            ..rename_config(&input)
        };
        let mut renamer = Renamer::new(config).unwrap();
        let results = renamer.run().unwrap();

        let statuses: Vec<RenameStatus> = results.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses
                .iter()
                .filter(|s| **s == RenameStatus::Preserved)
                .count(),
            2
        );
        assert!(statuses.contains(&RenameStatus::Renamed));
        assert!(statuses.contains(&RenameStatus::NoMetadata));

        // 标准名称未被修改
        assert!(input.join("IMG_20240115_143000.jpg").exists());
        assert!(input.join("MVIMG_20240115_143000_HDR.jpg").exists());
        // 非标准文件已重命名为标准 IMG 格式
        assert!(input.join("IMG_20240115_153000.jpg").exists());
        assert!(!input.join("DSC_0001.jpg").exists());

        let stats = renamer.stats();
        assert_eq!(stats.handled.load(Ordering::Relaxed), 4);
        assert_eq!(stats.preserved.load(Ordering::Relaxed), 2);
        assert_eq!(stats.renamed.load(Ordering::Relaxed), 1);
        assert_eq!(stats.no_metadata.load(Ordering::Relaxed), 1);

        // 未修改列表只包含无元数据文件，不含被保留的标准名称
        let list_path = dir.path().join("unmodified_files.txt");
        let count = write_unmodified_list(&list_path, &results).unwrap();
        assert_eq!(count, 1);
        let content = fs::read_to_string(&list_path).unwrap();
        assert!(content.contains("no_meta.jpg"));
        assert!(!content.contains("IMG_20240115_143000.jpg"));
        assert!(!content.contains("MVIMG_20240115_143000_HDR.jpg"));
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

        // 有 EXIF：重命名为标准 IMG 格式
        fs::write(input.join("DSC_0001.jpg"), exif_jpeg("2024:01:15 14:30:00")).unwrap();
        // 同一时间戳的第二张：冲突加 _1 后缀
        fs::write(input.join("DSC_0002.jpg"), exif_jpeg("2024:01:15 14:30:00")).unwrap();
        // 已经是标准统一格式：不修改
        fs::write(
            input.join("IMG_20240115_153000.jpg"),
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

        // 已重命名为标准 IMG 格式
        assert!(input.join("IMG_20240115_143000.jpg").exists());
        assert!(input.join("IMG_20240115_143000_1.jpg").exists());
        // 已是标准统一格式的原名仍在
        assert!(input.join("IMG_20240115_153000.jpg").exists());
        // 无元数据文件未动
        assert!(input.join("no_meta.jpg").exists());
        assert!(!input.join("DSC_0001.jpg").exists());
        assert!(!input.join("DSC_0002.jpg").exists());

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

        fs::write(input.join("DSC_0001.jpg"), exif_jpeg("2024:01:15 14:30:00")).unwrap();

        let config = Config {
            dry_run: true,
            ..rename_config(&input)
        };
        let mut renamer = Renamer::new(config).unwrap();
        let results = renamer.run().unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, RenameStatus::DryRun);
        assert!(input.join("DSC_0001.jpg").exists());
        assert!(!input.join("IMG_20240115_143000.jpg").exists());
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
