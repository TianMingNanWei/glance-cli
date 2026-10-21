mod cpu;
mod disk;
mod gpu;
mod host;
mod memory;
mod network;
mod process;

use crate::model::Snapshot;
use crate::soc::SocSampler;
pub use process::ProcessSampler;

pub struct Sampler {
    cpu: cpu::CpuSampler,
    disk: disk::DiskSampler,
    network: network::NetworkSampler,
    processes: ProcessSampler,
    soc: SocSampler,
    host: crate::model::HostInfo,
}

impl Sampler {
    pub fn new() -> Self {
        let mut sampler = Self {
            cpu: cpu::CpuSampler::new(),
            disk: disk::DiskSampler::new(),
            network: network::NetworkSampler::new(),
            processes: ProcessSampler::new(),
            soc: SocSampler::new(),
            host: host::current(),
        };
        // The first pass only stores counters. The pause gives the next pass a real delta.
        sampler.sample(true);
        std::thread::sleep(std::time::Duration::from_millis(300));
        sampler
    }

    pub fn sample(&mut self, include_processes: bool) -> Snapshot {
        self.host.uptime_seconds = host::uptime_seconds();
        let mut cpu = self.cpu.sample();
        let mut gpu = self.gpu_sample();
        let memory = memory::sample();
        let disk = self.disk.sample();
        let battery = disk::battery();
        let network = self.network.sample();
        let soc = self.soc.sample();
        cpu.power_watts = soc.cpu_watts;
        cpu.temperature_c = soc.temperature_c;
        for cluster in &mut cpu.clusters {
            if let Some(hertz) = soc.cluster_hz.get(&cluster.kind) {
                cluster.frequency_hz = Some(*hertz);
            }
            if let Some(watts) = soc.cluster_watts.get(&cluster.kind) {
                cluster.power_watts = Some(*watts);
            }
        }
        gpu.frequency_hz = soc.gpu_hz;
        gpu.power_watts = soc.gpu_watts;
        gpu.active = soc.gpu_active;
        if gpu.utilization.is_none() {
            gpu.utilization = soc.gpu_active;
        }
        let processes = if include_processes {
            self.processes.sample()
        } else {
            Vec::new()
        };
        Snapshot {
            host: self.host.clone(),
            cpu,
            gpu,
            npu: crate::model::Npu {
                supported: cfg!(target_arch = "aarch64"),
                power_watts: soc.ane_watts,
                active: soc.ane_active(),
            },
            memory,
            disk,
            battery,
            network,
            power: crate::model::Power {
                cpu_watts: soc.cpu_watts,
                gpu_watts: soc.gpu_watts,
                npu_watts: soc.ane_watts,
                memory_watts: soc.dram_watts,
                chip_watts: soc.chip_watts(),
            },
            processes,
        }
    }

    fn gpu_sample(&self) -> crate::model::Gpu {
        gpu::sample()
    }
}
