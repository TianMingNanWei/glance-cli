use crate::model::Process;
use std::collections::HashMap;
use std::ffi::CStr;
use std::time::Instant;

const PROC_PIDTASKINFO: i32 = 4;

#[repr(C)]
struct ProcTaskInfo {
    virtual_size: u64,
    resident_size: u64,
    total_user: u64,
    total_system: u64,
    threads_user: u64,
    threads_system: u64,
    policy: i32,
    faults: i32,
    pageins: i32,
    cow_faults: i32,
    messages_sent: i32,
    messages_received: i32,
    syscalls_mach: i32,
    syscalls_unix: i32,
    csw: i32,
    threadnum: i32,
    numrunning: i32,
    priority: i32,
}

unsafe extern "C" {
    fn proc_listallpids(buffer: *mut i32, size: i32) -> i32;
    fn proc_pidinfo(
        pid: i32,
        flavor: i32,
        arg: u64,
        buffer: *mut std::ffi::c_void,
        size: i32,
    ) -> i32;
    fn proc_name(pid: i32, buffer: *mut libc::c_char, size: u32) -> i32;
    fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32;
}

pub struct ProcessSampler {
    previous: HashMap<i32, (u64, Instant)>,
    bundles: HashMap<String, String>,
}

impl ProcessSampler {
    pub fn new() -> Self {
        Self {
            previous: HashMap::new(),
            bundles: HashMap::new(),
        }
    }

    pub fn sample(&mut self) -> Vec<Process> {
        let mut pids = vec![0i32; 4096];
        let bytes = unsafe {
            proc_listallpids(
                pids.as_mut_ptr(),
                (pids.len() * std::mem::size_of::<i32>()) as i32,
            )
        };
        if bytes <= 0 {
            return Vec::new();
        }
        let count = bytes as usize / std::mem::size_of::<i32>();
        let now = Instant::now();
        let mut records = Vec::new();
        for pid in pids.into_iter().take(count).filter(|pid| *pid > 0) {
            if let Some(record) = self.record(pid, now) {
                records.push(record);
            }
        }
        records.sort_by(|left, right| {
            right
                .cpu
                .partial_cmp(&left.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.previous
            .retain(|_, (_, seen)| now.duration_since(*seen).as_secs() < 30);
        records
    }

    fn record(&mut self, pid: i32, now: Instant) -> Option<Process> {
        let mut task = std::mem::MaybeUninit::<ProcTaskInfo>::zeroed();
        let size = std::mem::size_of::<ProcTaskInfo>() as i32;
        if unsafe { proc_pidinfo(pid, PROC_PIDTASKINFO, 0, task.as_mut_ptr().cast(), size) } != size
        {
            return None;
        }
        let task = unsafe { task.assume_init() };
        let cpu_time = task.total_user.wrapping_add(task.total_system);
        let fraction = if let Some((before, then)) = self.previous.get(&pid) {
            let elapsed = now.duration_since(*then).as_secs_f64();
            if elapsed > 0.0 {
                cpu_time.wrapping_sub(*before) as f64 / (elapsed * 1_000_000_000.0)
            } else {
                0.0
            }
        } else {
            0.0
        };
        self.previous.insert(pid, (cpu_time, now));
        let mut name = [0u8; 256];
        unsafe { proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
        let name = CStr::from_bytes_until_nul(&name)
            .ok()?
            .to_string_lossy()
            .into_owned();
        if name.is_empty() {
            return None;
        }
        Some(Process {
            pid,
            name,
            identifier: self.identifier(pid),
            threads: task.threadnum.max(0) as u32,
            cpu: fraction.max(0.0),
            memory_bytes: task.resident_size,
        })
    }

    fn identifier(&mut self, pid: i32) -> String {
        let mut buffer = vec![0u8; 4096];
        let length = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if length <= 0 {
            return String::new();
        }
        let path = CStr::from_bytes_until_nul(&buffer)
            .map(|text| text.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(range) = path.rfind(".app/Contents/MacOS/") else {
            return path;
        };
        let app = format!("{}.app", &path[..range]);
        if let Some(cached) = self.bundles.get(&app) {
            return cached.clone();
        }
        let identifier = crate::cf::bundle_identifier(&app).unwrap_or(path);
        self.bundles.insert(app, identifier.clone());
        identifier
    }
}
