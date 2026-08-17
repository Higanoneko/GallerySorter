//! Windows-specific operating system features.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::time::SystemTime;
use winapi::ctypes::c_void;
use winapi::shared::minwindef::FILETIME;
use winapi::um::fileapi::SetFileTime;
use winapi::um::handleapi::CloseHandle;
use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
use winapi::um::shellapi::ShellExecuteW;
use winapi::um::winnt::{HANDLE, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use winapi::um::winuser::SW_SHOW;

const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// 把创建时间与修改时间写回文件（访问时间保持不变）。
///
/// `std::fs::FileTimes` 与 `filetime` 均不支持设置创建时间，
/// 因此这里直接调用 WinAPI `SetFileTime`。
pub fn set_file_times(path: &Path, times: &super::FileTimes) -> io::Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let handle = file.as_raw_handle() as HANDLE;

    let creation_ft = times.created.map(to_filetime);
    let modified_ft = to_filetime(times.modified);

    let ok = unsafe {
        SetFileTime(
            handle,
            creation_ft
                .as_ref()
                .map_or(std::ptr::null(), |ft| ft as *const FILETIME),
            std::ptr::null(),
            &modified_ft as *const FILETIME,
        )
    };

    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `SystemTime` → `FILETIME`（自 1601-01-01 起 100ns 间隔计数）。
fn to_filetime(time: SystemTime) -> FILETIME {
    const UNIX_TO_FILETIME_TICKS: u64 = 116_444_736_000_000_000;
    let since_epoch = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = UNIX_TO_FILETIME_TICKS
        + since_epoch.as_secs() * 10_000_000
        + u64::from(since_epoch.subsec_nanos()) / 100;
    FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    }
}

/// Check if the current process is running with administrator privileges.
pub fn is_running_as_admin() -> bool {
    let mut token_handle: HANDLE = std::ptr::null_mut();
    let mut is_admin = false;

    // Try to open the process token
    let success = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) };

    if success != 0 && !token_handle.is_null() {
        let mut token_info: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
        let mut return_length: u32 = 0;

        let query_success = unsafe {
            winapi::um::securitybaseapi::GetTokenInformation(
                token_handle,
                TokenElevation,
                &mut token_info as *mut _ as *mut c_void,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut return_length,
            )
        };

        unsafe {
            CloseHandle(token_handle);
        }

        if query_success != 0 {
            is_admin = token_info.TokenIsElevated != 0;
        }
    }

    is_admin
}

/// Check if creating a symbolic link on Windows requires elevation.
/// Windows Symlink requires admin privileges unless Developer mode is enabled.
pub fn symlink_needs_elevation() -> bool {
    !is_running_as_admin()
}

/// Run the current executable with administrator privileges.
pub fn run_as_admin(args: &[String]) -> io::Result<()> {
    let exe_path = std::env::current_exe()?;

    // Build the command line
    let mut cmdline = String::new();
    for arg in args {
        if !cmdline.is_empty() {
            cmdline.push(' ');
        }
        if arg.contains(' ') || arg.contains('"') {
            cmdline.push('"');
            cmdline.push_str(&arg.replace('"', "\"\""));
            cmdline.push('"');
        } else {
            cmdline.push_str(arg);
        }
    }

    let operation: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let exe_path_utf16: Vec<u16> = exe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let params_utf16: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            exe_path_utf16.as_ptr(),
            if cmdline.is_empty() {
                std::ptr::null()
            } else {
                params_utf16.as_ptr()
            },
            std::ptr::null(),
            SW_SHOW,
        )
    };

    // ShellExecuteW returns a value > 32 on success
    if result as i32 > 32 {
        Ok(())
    } else {
        Err(io::Error::other("Failed to request elevation"))
    }
}

/// Re-execute the current process with elevated privileges for symlink operations.
pub fn elevate_for_symlink() -> io::Result<()> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|os| os.into_string().unwrap_or_default())
        .filter(|s| !s.is_empty())
        .collect();

    let mut elevated_args = vec!["--elevated-for-symlink".to_string()];
    elevated_args.extend(args);

    run_as_admin(&elevated_args)
}

/// Check if this process was started due to an elevation request.
pub fn was_started_for_elevation() -> bool {
    std::env::args().any(|arg| arg == "--elevated-for-symlink")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_filetime_known_values() {
        let unix_epoch = SystemTime::UNIX_EPOCH;
        let ft = to_filetime(unix_epoch);
        let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
        assert_eq!(ticks, 116_444_736_000_000_000);

        // 2020-01-01 00:00:00 UTC
        let known = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
        let ft = to_filetime(known);
        let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
        assert_eq!(ticks, 132_223_104_000_000_000);
    }
}
