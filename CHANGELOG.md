# Changelog

All notable changes to Glance CLI are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org).

## [0.1.1] - 2026-10-22

### Fixed

- `glance snapshot` and `glance snapshot --json` no longer panic with "Broken pipe" when piped into a command that stops reading early, such as `head`.

### Install

- Homebrew: `brew install tianmingnanwei/tap/glance`.

## [0.1.0] - 2026-10-21

The first release.

### Live view

- One screen, no paging, from 100×30 up: CPU, GPU, Power · NPU · Thermal, Memory, Network, Disk · Battery, and Processes.
- CPU load history, each cluster's frequency, power, and load, and every core with its own colored history.
- CPU, GPU, Neural Engine, and memory power, die temperature, and chip power history.
- Charger input against the adapter's rating, negotiated and live voltage and current, and battery charge or discharge power.
- Animated bars that ease between samples, shimmer, hold a falling peak, and pulse near full scale; `a` turns animations off.
- Search, sort by any column, and stop or force quit processes after confirming.
- Help (`?`) and About (`i`) pop-ups, mouse scrolling and selection, pause, and refresh from 250 ms to 5 s.

### Commands

- `glance snapshot` and `glance snapshot --json` print one reading.
- `glance about` and `glance --version`.

### Notes

- Nothing needs root. Fan speed is not shown, and GPU frequency appears only when a matching clock table exists.

[0.1.1]: https://github.com/TianMingNanWei/glance-cli/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/TianMingNanWei/glance-cli/releases/tag/v0.1.0
