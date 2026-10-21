# Glance CLI

Full-screen system monitor for macOS, in the spirit of btop. CPU cores, GPU, Neural Engine, memory, disk, battery, network, power, temperature, and processes stay on one screen.

```bash
cargo build --release
./target/release/glance
```

```text
glance                 live view
glance snapshot        print one reading
glance snapshot --json print one reading as JSON
```

In the live view, `q` quits, `c` sorts processes by CPU, and `m` sorts them by memory.

Readings come from the same public and system interfaces as the Mac app: host statistics, the GPU accelerator registry, IOReport energy and performance states, and the HID die-temperature sensors. Nothing here needs root. Fan speed is not reported. GPU frequency is reported only when a device-tree clock table matches the GPU's performance states.

Requires Rust 1.85 or later and macOS 14 or later.
