//! Best-effort peak resident-memory sampling for benchmark acceptance.
//!
//! No extra crates are pulled in (dependency additions are intentionally
//! avoided), so sampling is implemented per OS from what the standard library
//! and OS surfaces expose. When a platform has no dependency-free source, the
//! sampler returns `None` and the report records that peak memory was not
//! captured on that OS rather than reporting a misleading zero.

/// Return the process peak resident set size in bytes, if the current platform
/// exposes it without additional dependencies.
pub fn peak_resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux_peak_rss()
    }
    #[cfg(target_os = "macos")]
    {
        macos_peak_rss()
    }
    #[cfg(target_os = "windows")]
    {
        windows_peak_rss()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Human-readable description of the peak-memory source for this platform, so
/// acceptance reports are explicit about how the number was obtained (or why it
/// is absent).
pub fn peak_memory_source() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "linux:/proc/self/status:VmHWM"
    }
    #[cfg(target_os = "macos")]
    {
        "macos:libc::getrusage(RUSAGE_SELF).ru_maxrss"
    }
    #[cfg(target_os = "windows")]
    {
        "windows:K32GetProcessMemoryInfo.PeakWorkingSetSize"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "unknown:not-captured"
    }
}

#[cfg(target_os = "linux")]
fn linux_peak_rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        // VmHWM is the peak resident set size ("high water mark").
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kib * 1024);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn macos_peak_rss() -> Option<u64> {
    // Use libc::getrusage which is available without additional dependencies
    use std::mem::MaybeUninit;
    
    let mut usage = MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    
    if result == 0 {
        let usage = unsafe { usage.assume_init() };
        // On macOS, ru_maxrss is in bytes (unlike Linux where it's in KB)
        Some(usage.ru_maxrss as u64)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_peak_rss() -> Option<u64> {
    // Use Windows API without pulling in extra crates
    use std::mem::MaybeUninit;
    
    #[repr(C)]
    struct ProcessMemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }
    
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut ProcessMemoryCountersEx,
            cb: u32,
        ) -> i32;
    }
    
    unsafe {
        let mut counters = MaybeUninit::<ProcessMemoryCountersEx>::uninit();
        let counters_ptr = counters.as_mut_ptr();
        (*counters_ptr).cb = std::mem::size_of::<ProcessMemoryCountersEx>() as u32;
        
        let result = K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            counters_ptr,
            std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
        );
        
        if result != 0 {
            let counters = counters.assume_init();
            Some(counters.peak_working_set_size as u64)
        } else {
            None
        }
    }
}
