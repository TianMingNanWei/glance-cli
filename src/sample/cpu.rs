use crate::cf::{self, CFRelease};
use crate::model::{Cluster, Core, Cpu};
use crate::sample::host::{sysctl_string, sysctl_u32};
use std::ffi::CString;

const PROCESSOR_CPU_LOAD_INFO: i32 = 2;
const CPU_STATE_MAX: usize = 4;

#[link(name = "System")]
unsafe extern "C" {
    fn mach_host_self() -> u32;
    fn host_processor_info(
        host: u32,
        flavor: i32,
        out_count: *mut u32,
        out_info: *mut *mut i32,
        out_info_count: *mut u32,
    ) -> i32;
    fn mach_task_self() -> u32;
    fn vm_deallocate(target: u32, address: usize, size: usize) -> i32;
}

#[derive(Clone, Copy)]
struct Tick {
    user: u32,
    system: u32,
    idle: u32,
    nice: u32,
}

struct Topology {
    levels: Vec<u32>,
    names: Vec<String>,
    counts: Vec<u32>,
    kinds: Vec<String>,
}

pub struct CpuSampler {
    previous: Option<Vec<Tick>>,
    topology: Topology,
}

impl CpuSampler {
    pub fn new() -> Self {
        Self {
            previous: None,
            topology: Topology::current(),
        }
    }

    pub fn sample(&mut self) -> Cpu {
        let ticks = read_ticks().unwrap_or_default();
        let utilization = match &self.previous {
            Some(previous) => per_core(previous, &ticks),
            None => vec![0.0; ticks.len()],
        };
        self.previous = Some(ticks);
        self.topology.snapshot(&utilization)
    }
}

fn per_core(previous: &[Tick], current: &[Tick]) -> Vec<f64> {
    previous
        .iter()
        .zip(current)
        .map(|(before, after)| {
            let total = after.total().wrapping_sub(before.total());
            let busy = after.busy().wrapping_sub(before.busy());
            if total == 0 {
                0.0
            } else {
                (busy as f64 / total as f64).clamp(0.0, 1.0)
            }
        })
        .collect()
}

impl Tick {
    fn busy(self) -> u32 {
        self.user.wrapping_add(self.system).wrapping_add(self.nice)
    }
    fn total(self) -> u32 {
        self.busy().wrapping_add(self.idle)
    }
}

fn read_ticks() -> Option<Vec<Tick>> {
    let mut count = 0u32;
    let mut info: *mut i32 = std::ptr::null_mut();
    let mut info_count = 0u32;
    let result = unsafe {
        host_processor_info(
            mach_host_self(),
            PROCESSOR_CPU_LOAD_INFO,
            &mut count,
            &mut info,
            &mut info_count,
        )
    };
    if result != 0 || info.is_null() {
        return None;
    }
    let values = unsafe { std::slice::from_raw_parts(info, info_count as usize) };
    let mut ticks = Vec::with_capacity(count as usize);
    for cpu in 0..count as usize {
        let base = cpu * CPU_STATE_MAX;
        if base + 3 >= values.len() {
            break;
        }
        ticks.push(Tick {
            user: values[base] as u32,
            system: values[base + 1] as u32,
            idle: values[base + 2] as u32,
            nice: values[base + 3] as u32,
        });
    }
    let bytes = info_count as usize * std::mem::size_of::<i32>();
    unsafe { vm_deallocate(mach_task_self(), info as usize, bytes) };
    Some(ticks)
}

impl Topology {
    fn current() -> Self {
        let logical = sysctl_u32("hw.logicalcpu").unwrap_or(1) as usize;
        let level_count = sysctl_u32("hw.nperflevels").unwrap_or(1).max(1) as usize;
        let names: Vec<String> = (0..level_count)
            .map(|level| sysctl_string(&format!("hw.perflevel{level}.name")).unwrap_or_default())
            .collect();
        let counts: Vec<u32> = (0..level_count)
            .map(|level| sysctl_u32(&format!("hw.perflevel{level}.logicalcpu")).unwrap_or(0))
            .collect();
        let cluster_types = device_tree_types();
        let order = level_order(&cluster_types);
        let (levels, kinds) = if cluster_types.len() == logical && order.len() == level_count {
            let levels = cluster_types
                .iter()
                .map(|kind| order.iter().position(|item| item == kind).unwrap_or(0) as u32)
                .collect();
            (levels, order)
        } else if level_count > 1 {
            let mut levels = Vec::new();
            for level in (0..level_count).rev() {
                levels.extend(std::iter::repeat_n(
                    level as u32,
                    counts.get(level).copied().unwrap_or(0) as usize,
                ));
            }
            let kinds = if level_count == 2 {
                vec!["P".into(), "E".into()]
            } else {
                Vec::new()
            };
            (levels, kinds)
        } else {
            (
                vec![0; logical],
                vec![order.first().cloned().unwrap_or_else(|| "P".into())],
            )
        };
        let names = if names.is_empty() {
            vec![String::new()]
        } else {
            names
        };
        Self {
            levels,
            names,
            counts: if counts.is_empty() {
                vec![logical as u32]
            } else {
                counts
            },
            kinds,
        }
    }

    fn snapshot(&self, utilization: &[f64]) -> Cpu {
        let total = if utilization.is_empty() {
            0.0
        } else {
            utilization.iter().sum::<f64>() / utilization.len() as f64
        };
        let mut sums = vec![0.0; self.names.len()];
        let mut seen = vec![0u32; self.names.len()];
        for (index, value) in utilization.iter().enumerate() {
            let level = self.levels.get(index).copied().unwrap_or(0) as usize;
            if level < sums.len() {
                sums[level] += value;
                seen[level] += 1;
            }
        }
        let clusters = (0..self.names.len())
            .map(|level| Cluster {
                name: self.names[level].clone(),
                kind: self.kinds.get(level).cloned().unwrap_or_default(),
                core_count: self.counts.get(level).copied().unwrap_or(0),
                utilization: if seen[level] > 0 {
                    sums[level] / seen[level] as f64
                } else {
                    0.0
                },
                frequency_hz: None,
                power_watts: None,
            })
            .collect();
        let cores = utilization
            .iter()
            .enumerate()
            .map(|(index, value)| Core {
                id: index as u32,
                level: self.levels.get(index).copied().unwrap_or(0),
                utilization: *value,
            })
            .collect();
        Cpu {
            utilization: total,
            power_watts: None,
            temperature_c: None,
            clusters,
            cores,
        }
    }
}

fn device_tree_types() -> Vec<String> {
    let Ok(path) = CString::new("IODeviceTree:/cpus") else {
        return Vec::new();
    };
    let root = unsafe { cf::IORegistryEntryFromPath(0, path.as_ptr()) };
    if root == 0 {
        return Vec::new();
    }
    let mut iterator = 0u32;
    let plane = CString::new("IODeviceTree").unwrap();
    if unsafe { cf::IORegistryEntryGetChildIterator(root, plane.as_ptr(), &mut iterator) } != 0 {
        unsafe { cf::IOObjectRelease(root) };
        return Vec::new();
    }
    let mut found = Vec::new();
    loop {
        let entry = unsafe { cf::IOIteratorNext(iterator) };
        if entry == 0 {
            break;
        }
        let name = cf::entry_name(entry);
        if let Some(index) = name
            .strip_prefix("cpu")
            .and_then(|rest| rest.parse::<usize>().ok())
        {
            let key = cf::cf_string("cluster-type");
            let property =
                unsafe { cf::IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0) };
            unsafe { CFRelease(key) };
            let kind = cf::data_bytes(property)
                .map(|bytes| {
                    String::from_utf8_lossy(
                        &bytes
                            .into_iter()
                            .take_while(|byte| *byte != 0)
                            .collect::<Vec<_>>(),
                    )
                    .into_owned()
                })
                .unwrap_or_default();
            if !property.is_null() {
                unsafe { CFRelease(property) };
            }
            found.push((index, kind));
        }
        unsafe { cf::IOObjectRelease(entry) };
    }
    unsafe {
        cf::IOObjectRelease(iterator);
        cf::IOObjectRelease(root);
    }
    found.sort_by_key(|(index, _)| *index);
    found.into_iter().map(|(_, kind)| kind).collect()
}

fn level_order(types: &[String]) -> Vec<String> {
    fn rank(kind: &str) -> i32 {
        match kind.to_ascii_uppercase().as_str() {
            "S" => 4,
            "P" => 3,
            "M" => 2,
            "E" => 1,
            _ => 0,
        }
    }
    let mut unique = Vec::new();
    for kind in types {
        if !kind.is_empty() && !unique.contains(kind) {
            unique.push(kind.clone());
        }
    }
    unique.sort_by(|left, right| rank(right).cmp(&rank(left)).then_with(|| left.cmp(right)));
    unique
}
