mod about;
mod anim;
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
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") | Some("help") => print_help(),
        Some("-V") | Some("--version") | Some("version") => {
            println!("glance {}", about::VERSION)
        }
        Some("about") => print_about(),
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
  glance about           author, version, license, and source
  glance --version       print the version

Keys in the live view (press ? for all of them):
  q / Esc                quit
  /                      search processes
  c m p n t  r           sort by CPU, memory, PID, name, threads; reverse
  x / X                  stop / force quit the selected process
  space  + -  a          pause, refresh speed, animations"
    );
}

fn print_about() {
    let color = std::io::stdout().is_terminal();
    for (index, line) in about::BANNER.iter().enumerate() {
        if color {
            let (r, g, b) = [
                (90, 200, 255),
                (120, 180, 255),
                (150, 150, 255),
                (190, 120, 255),
                (230, 115, 225),
                (255, 110, 200),
            ][index];
            println!("\x1b[1;38;2;{r};{g};{b}m{line}\x1b[0m");
        } else {
            println!("{line}");
        }
    }
    println!();
    println!("{} — {}", about::NAME, about::DESCRIPTION);
    println!();
    for (label, value) in about::details() {
        println!("  {label:<11} {value}");
    }
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
    if let Some(battery) = &reading.battery {
        let state = if battery.charging {
            "charging"
        } else if battery.external_power {
            "on AC"
        } else {
            "on battery"
        };
        let flow = battery
            .battery_watts
            .map(|flow| format!("{flow:+.2} W"))
            .unwrap_or_else(|| "n/a".into());
        println!(
            "Battery {}  {state}  battery {flow}",
            percent(battery.fraction)
        );
        if let Some(charger) = &battery.charger {
            let optional = |value: Option<f64>, unit: &str, digits: usize| {
                value
                    .map(|value| format!("{value:.digits$} {unit}"))
                    .unwrap_or_else(|| "n/a".into())
            };
            println!(
                "Charger rated {} ({}, {})  input {} ({}, {})",
                optional(charger.rated_watts, "W", 0),
                optional(charger.rated_volts, "V", 1),
                optional(charger.rated_amps, "A", 2),
                optional(charger.input_watts, "W", 1),
                optional(charger.input_volts, "V", 2),
                optional(charger.input_amps, "A", 2),
            );
        }
    }
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
    let mut app = tui::App::new();
    let mut reading = sampler.sample(true);
    app.history.push(&reading, 0.3);
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut last = Instant::now();
    loop {
        app.refresh_progress = (last.elapsed().as_secs_f64() / app.interval.as_secs_f64()).min(1.0);
        app.anim.tick();
        terminal.draw(|frame| tui::draw(frame, &mut app, &reading))?;
        let wait = app
            .frame_interval()
            .min(app.interval.saturating_sub(last.elapsed()));
        if crossterm::event::poll(wait)? {
            use crossterm::event::{Event, KeyEventKind};
            match crossterm::event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key, &reading) {
                        break;
                    }
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse, &reading),
                _ => {}
            }
        }
        if !app.paused && last.elapsed() >= app.interval {
            let seconds = last.elapsed().as_secs_f64();
            reading = sampler.sample(true);
            app.history.push(&reading, seconds);
            last = Instant::now();
        }
    }
    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture);
        ratatui::restore();
    }
}
