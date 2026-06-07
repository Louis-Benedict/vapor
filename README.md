<p align="center">
  <img src="logo.svg" width="180" alt="Vapor logo" />
</p>

# Vapor

A minimal macOS menu bar app that shows system stats. Written in Rust.

I built this because every stat app I tried seems to hog a lot of resources themselves - I wanted an app that does not peek at me when I inspect my resource monitor. Vapor does this in a few hundred lines of Rust with a fraction of the footprint of other apps.

> **Note:** Only tested on Apple M5 Pro. Apple Silicon is required for the GPU temperature and usage readings (they rely on SMC keys and IOAccelerator properties that differ on Intel).

---

## Resource usage

Approximate idle footprint compared to popular alternatives:

| App | Memory | CPU (idle) | Size on disk |
|---|---|---|---|
| iStatMenus | ~100–200 MB | 3–8% | 66 MB |
| Stats (open source) | ~100–200 MB | 2–6% | 42 MB |
| **Vapor** | **~20 MB** | **~0.3%** | **500 KB** |

Vapor polls every 5 seconds and does nothing in between. No background threads, no telemetry, no Electron.

---

## How it works

**Temperatures** — read directly from the System Management Controller (SMC) via IOKit. On first launch Vapor enumerates all SMC keys once and caches the ones that look like CPU (`Tp*`) or GPU (`Tg*`) sensors. Every poll after that is a handful of IOKit struct calls against the cached key list.

**CPU usage** — `host_statistics` with `HOST_CPU_LOAD_INFO` returns cumulative tick counters (user / system / idle / nice). Vapor diffs two consecutive snapshots to get a percentage.

**RAM usage** — `host_statistics64` with `HOST_VM_INFO64` gives page counts for active, wired, and compressed memory. Multiplied by page size and divided by total physical memory from `sysctl`.

**GPU usage** — iterates `IOAccelerator` services via IOKit and reads the `PerformanceStatistics` dictionary, specifically the `Device Utilization %` key that Apple Silicon exposes.

All of this runs on the main thread inside a winit event loop set to `ActivationPolicy::Accessory` (no Dock icon, no app switcher entry).

---

## Toggle menu

Click the menu bar title to open a dropdown. Each stat has a checkmark item — uncheck it and Vapor skips that system query entirely until you turn it back on.

---

## Running without a terminal

Install as a launchd agent so it starts automatically at login:

```xml
<!-- ~/Library/LaunchAgents/com.Vapor.plist -->
<key>ProgramArguments</key>
<array>
    <string>/path/to/Vapor/target/release/Vapor</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
```

```sh
launchctl load ~/Library/LaunchAgents/com.Vapor.plist
```

---

## Build

```sh
cargo build --release
```

Requires Xcode command line tools for the IOKit and CoreFoundation framework links.
