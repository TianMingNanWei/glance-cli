use crate::model::HostInfo;
use std::ffi::CString;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn current() -> HostInfo {
    HostInfo {
        host_name: sysctl_string("kern.hostname").unwrap_or_default(),
        chip: sysctl_string("machdep.cpu.brand_string").unwrap_or_else(|| "Unknown".into()),
        model: sysctl_string("hw.model").unwrap_or_default(),
        os_version: sysctl_string("kern.osproductversion").unwrap_or_default(),
        os_build: sysctl_string("kern.osversion").unwrap_or_default(),
        logical_cores: sysctl_u32("hw.logicalcpu").unwrap_or(1),
        memory_bytes: sysctl_u64("hw.memsize").unwrap_or(0),
        uptime_seconds: uptime_seconds(),
    }
}

pub fn uptime_seconds() -> u64 {
    let Some(boot) = boot_unix() else { return 0 };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    now.saturating_sub(boot)
}

fn boot_unix() -> Option<u64> {
    let mut time = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let mut size = std::mem::size_of::<libc::timeval>();
    let name = CString::new("kern.boottime").ok()?;
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut time as *mut libc::timeval).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result == 0 {
        Some(time.tv_sec as u64)
    } else {
        None
    }
}

pub fn sysctl_string(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut size = 0usize;
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || size <= 1
    {
        return None;
    }
    let mut buffer = vec![0u8; size];
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    Some(String::from_utf8_lossy(&buffer[..size.saturating_sub(1)]).into_owned())
}

pub fn sysctl_u32(name: &str) -> Option<u32> {
    sysctl_value(name)
}

pub fn sysctl_u64(name: &str) -> Option<u64> {
    sysctl_value(name)
}

fn sysctl_value<T: Default>(name: &str) -> Option<T> {
    let name = CString::new(name).ok()?;
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>();
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut T).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result == 0 {
        Some(value)
    } else {
        None
    }
}
