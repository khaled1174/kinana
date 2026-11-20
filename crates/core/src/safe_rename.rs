//! Exclusive OS renames only; operation-specific transactions stay separate.
use std::path::Path;

/// Atomically swap two existing directory entries on the same filesystem.
/// Neither name is ever absent and nothing is overwritten: each name ends up
/// holding what the other held. Final symlinks are swapped, not followed.
pub(crate) fn exchange(first: &Path, second: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let first = CString::new(first.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in file path")
        })?;
        let second = CString::new(second.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in file path")
        })?;
        #[cfg(target_os = "macos")]
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        #[cfg(target_os = "linux")]
        unsafe extern "C" {
            fn renameat2(
                from_dir: i32,
                from: *const std::ffi::c_char,
                to_dir: i32,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        // RENAME_SWAP (macOS) and RENAME_EXCHANGE (Linux) fail unless both
        // names exist. CString pointers are valid and NUL-terminated for the
        // duration of this call.
        #[cfg(target_os = "macos")]
        let result = unsafe { renamex_np(first.as_ptr(), second.as_ptr(), 0x2) };
        #[cfg(target_os = "linux")]
        let result = unsafe { renameat2(-100, first.as_ptr(), -100, second.as_ptr(), 2) };
        if result == 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // No atomic exchange is used on other systems; callers refuse instead
        // of replacing in two steps.
        let _ = (first, second);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in file path")
        })?;
        let target = CString::new(target.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in file path")
        })?;
        #[cfg(target_os = "macos")]
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        #[cfg(target_os = "linux")]
        unsafe extern "C" {
            fn renameat2(
                from_dir: i32,
                from: *const std::ffi::c_char,
                to_dir: i32,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        // Both OS calls atomically refuse an existing destination. CString
        // pointers are valid and NUL-terminated for the duration of this call.
        #[cfg(target_os = "macos")]
        let result = unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 0x4) };
        #[cfg(target_os = "linux")]
        let result = unsafe { renameat2(-100, source.as_ptr(), -100, target.as_ptr(), 1) };
        if result == 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        let source = source
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let target = target
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        // No MOVEFILE_REPLACE_EXISTING flag is supplied.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } != 0 {
            return Ok(());
        }
        return Err(std::io::Error::last_os_error());
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}
