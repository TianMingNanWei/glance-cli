use crate::model::Network;
use std::time::Instant;

#[repr(C)]
struct IfData {
    ifi_type: u8,
    ifi_typelen: u8,
    ifi_physical: u8,
    ifi_addrlen: u8,
    ifi_hdrlen: u8,
    ifi_recvquota: u8,
    ifi_xmitquota: u8,
    ifi_unused1: u8,
    ifi_mtu: u32,
    ifi_metric: u32,
    ifi_baudrate: u32,
    ifi_ipackets: u32,
    ifi_ierrors: u32,
    ifi_opackets: u32,
    ifi_oerrors: u32,
    ifi_collisions: u32,
    ifi_ibytes: u32,
    ifi_obytes: u32,
}

pub struct NetworkSampler {
    previous_in: Option<u64>,
    previous_out: Option<u64>,
    previous: Option<Instant>,
}

impl NetworkSampler {
    pub fn new() -> Self {
        Self {
            previous_in: None,
            previous_out: None,
            previous: None,
        }
    }

    pub fn sample(&mut self) -> Network {
        let (inbound, outbound) = counters();
        let now = Instant::now();
        let rates = match (self.previous_in, self.previous_out, self.previous) {
            (Some(before_in), Some(before_out), Some(before)) => {
                let elapsed = now.duration_since(before).as_secs_f64();
                if elapsed > 0.0 {
                    (
                        inbound.wrapping_sub(before_in) as f64 / elapsed,
                        outbound.wrapping_sub(before_out) as f64 / elapsed,
                    )
                } else {
                    (0.0, 0.0)
                }
            }
            _ => (0.0, 0.0),
        };
        self.previous_in = Some(inbound);
        self.previous_out = Some(outbound);
        self.previous = Some(now);
        Network {
            download_bytes_per_second: rates.0,
            upload_bytes_per_second: rates.1,
        }
    }
}

fn counters() -> (u64, u64) {
    let mut pointer: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut pointer) } != 0 || pointer.is_null() {
        return (0, 0);
    }
    let mut inbound = 0u64;
    let mut outbound = 0u64;
    let mut cursor = pointer;
    while !cursor.is_null() {
        let interface = unsafe { &*cursor };
        let name = unsafe { std::ffi::CStr::from_ptr(interface.ifa_name) }.to_string_lossy();
        let up = interface.ifa_flags & (libc::IFF_UP as u32) != 0;
        let link = !interface.ifa_addr.is_null()
            && unsafe { (*interface.ifa_addr).sa_family } == libc::AF_LINK as u8;
        if name != "lo0" && up && link && !interface.ifa_data.is_null() {
            let data = unsafe { &*(interface.ifa_data as *const IfData) };
            inbound += data.ifi_ibytes as u64;
            outbound += data.ifi_obytes as u64;
        }
        cursor = interface.ifa_next;
    }
    unsafe { libc::freeifaddrs(pointer) };
    (inbound, outbound)
}
