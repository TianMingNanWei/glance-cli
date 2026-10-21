use crate::model::Memory;
use crate::sample::host::{sysctl_u32, sysctl_u64};
use std::ffi::CString;

const HOST_VM_INFO64: i32 = 4;

#[repr(C)]
struct VmStatistics64 {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purges: u64,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u64,
    compressions: u64,
    swapins: u64,
    swapouts: u64,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u64,
}

#[repr(C)]
struct XswUsage {
    total: u64,
    used: u64,
    avail: u64,
    pagesize: u32,
    encrypted: i32,
}

#[link(name = "System")]
unsafe extern "C" {
    fn mach_host_self() -> u32;
    fn host_statistics64(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
}

pub fn sample() -> Memory {
    let page = sysctl_u64("hw.pagesize").unwrap_or(16_384);
    let total = sysctl_u64("hw.memsize").unwrap_or(0);
    let mut stats = std::mem::MaybeUninit::<VmStatistics64>::zeroed();
    let mut count = (std::mem::size_of::<VmStatistics64>() / std::mem::size_of::<i32>()) as u32;
    let result = unsafe {
        host_statistics64(
            mach_host_self(),
            HOST_VM_INFO64,
            stats.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if result != 0 {
        return empty(total);
    }
    let stats = unsafe { stats.assume_init() };
    let pages = |count: u32| count as u64 * page;
    let used_pages =
        stats.active_count as u64 + stats.wire_count as u64 + stats.compressor_page_count as u64;
    Memory {
        total_bytes: total,
        used_bytes: (used_pages * page).min(total),
        wired_bytes: pages(stats.wire_count),
        active_bytes: pages(stats.active_count),
        inactive_bytes: pages(stats.inactive_count),
        compressed_bytes: pages(stats.compressor_page_count),
        free_bytes: pages(stats.free_count),
        swap_bytes: swap_used(),
        pressure: pressure(),
    }
}

fn empty(total: u64) -> Memory {
    Memory {
        total_bytes: total,
        used_bytes: 0,
        wired_bytes: 0,
        active_bytes: 0,
        inactive_bytes: 0,
        compressed_bytes: 0,
        free_bytes: 0,
        swap_bytes: 0,
        pressure: "n/a".into(),
    }
}

fn swap_used() -> u64 {
    let Ok(name) = CString::new("vm.swapusage") else {
        return 0;
    };
    let mut usage = std::mem::MaybeUninit::<XswUsage>::zeroed();
    let mut size = std::mem::size_of::<XswUsage>();
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            usage.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result == 0 {
        unsafe { usage.assume_init().used }
    } else {
        0
    }
}

fn pressure() -> String {
    match sysctl_u32("kern.memorystatus_vm_pressure_level") {
        Some(level) if level >= 4 => "critical".into(),
        Some(level) if level >= 2 => "warning".into(),
        Some(_) => "normal".into(),
        None => "n/a".into(),
    }
}
