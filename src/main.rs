mod cf;
mod format;
mod model;
mod sample;
mod soc;
mod tui;

use crate::format::{bytes, bytes_per_second, celsius, hertz, percent, watts};
use crate::model::Snapshot;
use crate::sample::Sampler;
use std::io::{self, IsTerminal};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") | Some("help") => print_help(),
        Some("snapshot") => snapshot(args.iter().any(|arg| arg == "--json")),
        Some(other) => {
            eprintln!("unknown command '{other}'. Try `glance --help`.");
            std::process::exit(2);
        }
        None => {
            if let Err(error) = live() {
                eprintln!("glance: {error}");
                std::process::exit(1);
            }
        }
    }
}

fn print_help() {
    println!(
        "\
glance — full-screen system monitor

Usage:
  glance                 live view, one screen, no paging
  glance snapshot        print one reading
  glance snapshot --json print one reading as JSON

Keys in the live view:
  q                      quit
  c                      sort processes by CPU
  m                      sort processes by memory"
    );
}

fn snapshot(json: bool) {
    let mut sampler = Sampler::new();
    let reading = sampler.sample(true);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reading).unwrap_or_else(|_| "{}".into())
        );
        return;
    }
    print_reading(&reading);
}

fn print_reading(reading: &Snapshot) {
    let host = &reading.host;
    println!(
        "{}  {}  {}  macOS {} ({})",
        host.host_name, host.chip, host.model, host.os_version, host.os_build
    );
    println!(
        "CPU {}  {}  {}",
        percent(reading.cpu.utilization),
        reading
            .cpu
            .power_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .cpu
            .temperature_c
            .map(celsius)
            .unwrap_or_else(|| "n/a".into())
    );
    for cluster in &reading.cpu.clusters {
        let name = if cluster.name.is_empty() {
            cluster.kind.as_str()
        } else {
            cluster.name.as_str()
        };
        println!(
            "  {name} ×{}  {}  {}  {}",
            cluster.core_count,
            percent(cluster.utilization),
            cluster
                .frequency_hz
                .map(hertz)
                .unwrap_or_else(|| "n/a".into()),
            cluster
                .power_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into())
        );
    }
    println!(
        "GPU {}  {}  {}  {}",
        reading
            .gpu
            .utilization
            .map(percent)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .gpu
            .memory_bytes
            .map(bytes)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .gpu
            .frequency_hz
            .map(hertz)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .gpu
            .power_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into())
    );
    let npu = match reading.npu.active {
        Some(true) => "active",
        Some(false) => "idle",
        None => "n/a",
    };
    println!(
        "NPU {npu}  {}",
        reading
            .npu
            .power_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Power CPU {}  GPU {}  NPU {}  memory {}  chip {}",
        reading
            .power
            .cpu_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .power
            .gpu_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .power
            .npu_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .power
            .memory_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into()),
        reading
            .power
            .chip_watts
            .map(watts)
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Memory {} / {}  pressure {}",
        bytes(reading.memory.used_bytes),
        bytes(reading.memory.total_bytes),
        reading.memory.pressure
    );
    println!(
        "Network ↓ {}  ↑ {}",
        bytes_per_second(reading.network.download_bytes_per_second),
        bytes_per_second(reading.network.upload_bytes_per_second)
    );
    println!("Processes {}", reading.processes.len());
    for process in reading.processes.iter().take(8) {
        println!(
            "  {:<6} {:<16} {:>8} {:>8}  {}",
            process.pid,
            process.name,
            bytes(process.memory_bytes),
            percent(process.cpu),
            process.identifier
        );
    }
}

fn live() -> io::Result<()> {
    if !io::stdout().is_terminal() {
        eprintln!("glance needs a terminal. Use `glance snapshot` to print one reading.");
        std::process::exit(1);
    }
    let mut sampler = Sampler::new();
    let mut history = tui::History::default();
    let mut reading = sampler.sample(true);
    history.push(&reading);
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    let mut last = Instant::now();
    loop {
        terminal.draw(|frame| tui::draw(frame, &reading, &history))?;
        if crossterm::event::poll(Duration::from_millis(200))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                use crossterm::event::{KeyCode, KeyModifiers};
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('c') => sampler.set_sort_by_memory(false),
                    KeyCode::Char('m') => sampler.set_sort_by_memory(true),
                    _ => {}
                }
            }
        }
        if last.elapsed() >= Duration::from_secs(1) {
            reading = sampler.sample(true);
            history.push(&reading);
            last = Instant::now();
        }
    }
    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
