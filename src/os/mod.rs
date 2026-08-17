//! Platform-specific module for operating system features.

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

#[cfg(windows)]
pub mod windows;

/// 文件时间快照：创建时间（Windows）在多数 Unix 平台上不可用，故为 `Option`。
#[derive(Debug, Clone, Copy)]
pub struct FileTimes {
    /// 创建时间；Unix 上通常无法读取，为 `None`。
    pub created: Option<SystemTime>,
    /// 修改时间。
    pub modified: SystemTime,
}

/// 读取源文件的创建时间与修改时间；必须在删除源文件之前调用。
pub fn read_file_times(path: &Path) -> io::Result<FileTimes> {
    let metadata = fs::metadata(path)?;
    Ok(FileTimes {
        created: metadata.created().ok(),
        modified: metadata.modified()?,
    })
}

/// 把快照时间写回文件：Windows 还原创建时间与修改时间；其他平台仅还原修改时间。
#[cfg(windows)]
pub fn restore_file_times(path: &Path, times: &FileTimes) -> io::Result<()> {
    windows::set_file_times(path, times)
}

/// 把快照时间写回文件：Unix 无创建时间概念，只还原修改时间。
#[cfg(not(windows))]
pub fn restore_file_times(path: &Path, times: &FileTimes) -> io::Result<()> {
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(times.modified))
}

/// Check if the current process has administrator privileges.
#[cfg(unix)]
pub fn has_admin_privileges() -> bool {
    // On Unix, check if EUID is 0 (root)
    nix::unistd::geteuid().is_root()
}

/// Check if the current process has administrator privileges.
#[cfg(windows)]
pub fn has_admin_privileges() -> bool {
    windows::is_running_as_admin()
}

/// Request elevation and restart the current process with admin privileges.
#[cfg(unix)]
pub fn request_elevation(_args: &[String]) -> std::io::Result<()> {
    // On Unix, sudo can be used but it's not automatic
    // User needs to run with sudo manually
    Ok(())
}

/// Request elevation and restart the current process with admin privileges.
#[cfg(windows)]
pub fn request_elevation(args: &[String]) -> std::io::Result<()> {
    windows::run_as_admin(args)
}

/// Get the platform-specific symlink implementation function.
/// Returns None if the platform doesn't support the symlink operation directly.
#[cfg(windows)]
pub fn needs_elevation_for_symlink() -> bool {
    windows::symlink_needs_elevation()
}

#[cfg(unix)]
pub fn needs_elevation_for_symlink() -> bool {
    false
}
