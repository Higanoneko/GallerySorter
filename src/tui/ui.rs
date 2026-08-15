//! UI 渲染与执行逻辑
//!
//! 负责渲染调度与处理执行过程。

use crate::process::{FileResult, ProcessingStats, ProcessingStatus};
use crate::rename::{RenameResult, RenameStatus, Renamer, write_unmodified_list};
use crate::tui::event::{EventPoll, TuiEvent};
use crate::tui::screens;
use crate::tui::state::{AppState, ProgressState, Screen, SummaryState};
use crate::tui::theme::{config::PROGRESS_RENDER_THROTTLE, theme};
use ratatui::{DefaultTerminal, Frame, buffer::Buffer, layout::Rect, style::Style};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 在 TUI 内执行处理流程
pub fn run_processing(
    terminal: &mut DefaultTerminal,
    config: crate::config::Config,
    log_path: Option<std::path::PathBuf>,
) -> std::io::Result<SummaryState> {
    if config.unify_filenames {
        return run_rename_processing(terminal, config, log_path);
    }
    run_organize_processing(terminal, config, log_path)
}

/// 后台处理上下文（进度线程共享状态 + 结果通道）
struct ProcessingBackground {
    stats: Arc<ProcessingStats>,
    rename_stats: Option<Arc<crate::rename::RenameStats>>,
    cancel: Arc<AtomicBool>,
    total_files: usize,
    rx: std::sync::mpsc::Receiver<Result<(), ()>>,
    handle: std::thread::JoinHandle<(ProcessingStats, Vec<FileResult>)>,
}

/// 共享进度循环：渲染进度屏 → 监听取消 → 等待后台线程 → 汇总结果
fn run_with_background(
    terminal: &mut DefaultTerminal,
    config: &crate::config::Config,
    log_path: Option<std::path::PathBuf>,
    background: ProcessingBackground,
) -> std::io::Result<SummaryState> {
    let mut state = AppState {
        current_screen: Screen::Progress,
        progress_state: ProgressState::new(background.stats.clone(), background.total_files),
        ..Default::default()
    };

    render(terminal, &mut state)?;

    std::thread::sleep(Duration::from_millis(100));
    render(terminal, &mut state)?;

    let event_poll = EventPoll::default();
    let mut frame_index: u64 = 0;
    loop {
        // 进度屏监听 Esc/q：置位取消标志，立即停止调度新文件（ADR-0003）
        match event_poll.next() {
            event if is_cancel_event(event.clone()) => {
                background.cancel.store(true, Ordering::Relaxed);
            }
            _ => {}
        }

        if let Ok(Ok(())) = background.rx.recv_timeout(Duration::from_millis(50)) {
            break;
        }

        // 统一化模式下把 RenameStats 同步到进度统计，保证进度实时更新
        if let Some(rename_stats) = &background.rename_stats {
            sync_rename_progress(&background.stats, rename_stats);
        }

        frame_index += 1;
        // 渲染节流：非必须帧跳过全帧重绘
        if !should_skip_render(frame_index, PROGRESS_RENDER_THROTTLE) {
            render(terminal, &mut state)?;
        }
    }

    let (final_stats, results) = background.handle.join().unwrap();
    let cancelled = background.cancel.load(Ordering::Relaxed);

    Ok(SummaryState::new_with_status(
        final_stats,
        results,
        config.dry_run,
        log_path,
        cancelled,
    ))
}

/// 归档流水线（Processor）
fn run_organize_processing(
    terminal: &mut DefaultTerminal,
    config: crate::config::Config,
    log_path: Option<std::path::PathBuf>,
) -> std::io::Result<SummaryState> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut processor =
        match crate::process::Processor::new_with_cancel(config.clone(), cancel.clone()) {
            Ok(p) => p,
            Err(_) => {
                let stats = ProcessingStats::new();
                return Ok(SummaryState::new(
                    stats,
                    Vec::new(),
                    config.dry_run,
                    log_path,
                ));
            }
        };

    let total_files = processor.total_files_count().unwrap_or(0);
    let stats = processor.stats_arc();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), ()>>();

    let handle = std::thread::spawn(move || {
        let results = processor.run().unwrap_or_default();
        let final_stats = (*processor.stats()).clone();

        let _ = tx.send(Ok(()));

        (final_stats, results)
    });

    run_with_background(
        terminal,
        &config,
        log_path,
        ProcessingBackground {
            stats,
            rename_stats: None,
            cancel,
            total_files,
            rx,
            handle,
        },
    )
}

/// 文件名统一化流水线（Renamer）
fn run_rename_processing(
    terminal: &mut DefaultTerminal,
    config: crate::config::Config,
    log_path: Option<std::path::PathBuf>,
) -> std::io::Result<SummaryState> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut renamer = match Renamer::new_with_cancel(config.clone(), cancel.clone()) {
        Ok(r) => r,
        Err(_) => {
            let stats = ProcessingStats::new();
            return Ok(SummaryState::new(
                stats,
                Vec::new(),
                config.dry_run,
                log_path,
            ));
        }
    };

    let total_files = renamer.total_files_count().unwrap_or(0);
    let progress_stats = Arc::new(ProcessingStats::new());
    let rename_stats = renamer.stats_arc();
    let report_path = config.get_unmodified_list_file();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), ()>>();

    let handle = std::thread::spawn(move || {
        let results = renamer.run().unwrap_or_default();
        if !config.dry_run
            && let Err(e) = write_unmodified_list(&report_path, &results)
        {
            tracing::warn!(error = %e, "Failed to write unmodified files list");
        }
        let final_stats = to_processing_stats(renamer.stats());

        let _ = tx.send(Ok(()));

        (final_stats, to_file_results(results))
    });

    run_with_background(
        terminal,
        &config,
        log_path,
        ProcessingBackground {
            stats: progress_stats,
            rename_stats: Some(rename_stats),
            cancel,
            total_files,
            rx,
            handle,
        },
    )
}

/// 把 RenameStats 映射为 ProcessingStats（TUI 摘要复用现有统计展示）
fn to_processing_stats(stats: &crate::rename::RenameStats) -> ProcessingStats {
    let result = ProcessingStats::new();
    result
        .total_files
        .store(stats.total_files.load(Ordering::Relaxed), Ordering::Relaxed);
    result
        .processed
        .store(stats.renamed.load(Ordering::Relaxed), Ordering::Relaxed);
    result.skipped.store(
        stats.already_unified.load(Ordering::Relaxed) + stats.no_metadata.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    result
        .failed
        .store(stats.failed.load(Ordering::Relaxed), Ordering::Relaxed);
    result
}

/// 把 RenameStats 同步到 ProcessingStats 进度统计（进度屏实时显示）
///
/// 进度条按“已处理”文件数推进；已统一 / 无元数据 / 失败都算已处理，
/// 最终摘要由 [`to_processing_stats`] 重新映射。
fn sync_rename_progress(progress: &ProcessingStats, rename_stats: &crate::rename::RenameStats) {
    progress.total_files.store(
        rename_stats.total_files.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    progress.processed.store(
        rename_stats.handled.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    progress.skipped.store(0, Ordering::Relaxed);
    progress.failed.store(
        rename_stats.failed.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

/// 把 RenameResult 映射为 FileResult（TUI 摘要复用现有结果展示）
fn to_file_results(results: Vec<RenameResult>) -> Vec<FileResult> {
    results
        .into_iter()
        .map(|r| FileResult {
            source: r.source,
            destination: r.destination,
            time_info: None,
            status: match r.status {
                RenameStatus::Renamed => ProcessingStatus::Success,
                RenameStatus::DryRun => ProcessingStatus::DryRun,
                RenameStatus::AlreadyUnified | RenameStatus::NoMetadata => {
                    ProcessingStatus::Skipped
                }
                RenameStatus::Failed => ProcessingStatus::Failed,
                RenameStatus::Cancelled => ProcessingStatus::Cancelled,
            },
            error: r.error,
        })
        .collect()
}

/// 进度屏取消事件判定（Esc / q / Q）
pub fn is_cancel_event(event: TuiEvent) -> bool {
    matches!(
        event,
        TuiEvent::Escape | TuiEvent::Char('q') | TuiEvent::Char('Q')
    )
}

/// 进度屏渲染节流决策：给定帧序号与节流间隔，判断是否跳过本帧
///
/// 间隔为 0 表示不节流（每帧都渲染）。
pub fn should_skip_render(frame_index: u64, throttle_interval: u64) -> bool {
    throttle_interval > 0 && !frame_index.is_multiple_of(throttle_interval)
}

/// 设置背景色
fn set_background(area: Rect, buf: &mut Buffer) {
    let style = Style::new().bg(theme().bg);
    buf.set_style(area, style);
}

/// 渲染应用
pub fn render(terminal: &mut DefaultTerminal, state: &mut AppState) -> std::io::Result<()> {
    terminal.draw(|frame| draw(frame, frame.area(), state))?;
    Ok(())
}

fn draw(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let buf = frame.buffer_mut();
    set_background(area, buf);

    match state.current_screen {
        Screen::MainMenu => screens::main_menu::draw(frame, area, state),
        Screen::ConfigWizard => screens::config_wizard::draw(frame, area, state),
        Screen::Progress => screens::progress::draw(frame, area, state),
        Screen::Summary => screens::summary::draw(frame, area, state),
        Screen::Exit => screens::exit::draw(frame, area),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_is_cancel_event() {
        assert!(is_cancel_event(TuiEvent::Escape));
        assert!(is_cancel_event(TuiEvent::Char('q')));
        assert!(is_cancel_event(TuiEvent::Char('Q')));
        assert!(!is_cancel_event(TuiEvent::Enter));
        assert!(!is_cancel_event(TuiEvent::Char('a')));
        assert!(!is_cancel_event(TuiEvent::None));
    }

    #[test]
    fn test_should_skip_render() {
        // 节流间隔 2：奇数帧跳过，偶数帧渲染
        assert!(should_skip_render(1, 2));
        assert!(!should_skip_render(2, 2));
        assert!(should_skip_render(3, 2));
        assert!(!should_skip_render(4, 2));
        // 间隔 0：不节流，每帧渲染
        assert!(!should_skip_render(1, 0));
        assert!(!should_skip_render(10, 0));
        // 第一帧不跳过
        assert!(!should_skip_render(0, 2));
    }

    #[test]
    fn test_sync_rename_progress_maps_rename_stats() {
        let rename_stats = crate::rename::RenameStats {
            total_files: AtomicUsize::new(4),
            handled: AtomicUsize::new(4),
            renamed: AtomicUsize::new(2),
            already_unified: AtomicUsize::new(1),
            no_metadata: AtomicUsize::new(1),
            failed: AtomicUsize::new(0),
        };

        // 最终摘要：processed = 实际重命名数，skipped = 已统一 + 无元数据
        let final_stats = to_processing_stats(&rename_stats);
        assert_eq!(final_stats.processed.load(Ordering::Relaxed), 2);
        assert_eq!(final_stats.skipped.load(Ordering::Relaxed), 2);

        // 实时进度：processed = 已处理数（保证进度条走满）
        let live = ProcessingStats::new();
        sync_rename_progress(&live, &rename_stats);
        assert_eq!(live.processed.load(Ordering::Relaxed), 4);
        assert_eq!(live.skipped.load(Ordering::Relaxed), 0);
        assert_eq!(live.total_files.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn test_to_file_results_maps_rename_statuses() {
        let results = vec![
            RenameResult {
                source: std::path::PathBuf::from("a.jpg"),
                destination: Some(std::path::PathBuf::from("20240115_143000.jpg")),
                status: RenameStatus::Renamed,
                error: None,
            },
            RenameResult {
                source: std::path::PathBuf::from("b.jpg"),
                destination: None,
                status: RenameStatus::NoMetadata,
                error: None,
            },
            RenameResult {
                source: std::path::PathBuf::from("c.jpg"),
                destination: None,
                status: RenameStatus::Failed,
                error: Some("boom".into()),
            },
        ];

        let mapped = to_file_results(results);

        assert_eq!(mapped[0].status, ProcessingStatus::Success);
        assert_eq!(
            mapped[0].destination,
            Some(std::path::PathBuf::from("20240115_143000.jpg"))
        );
        assert_eq!(mapped[1].status, ProcessingStatus::Skipped);
        assert_eq!(mapped[2].status, ProcessingStatus::Failed);
        assert_eq!(mapped[2].error.as_deref(), Some("boom"));
    }
}
