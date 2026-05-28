# BitRufus

A macOS torrent client with a Rust core and SwiftUI frontend, bridged via UniFFI.

## Prerequisites

- macOS 13.0+
- Xcode (latest stable)
- [Rust toolchain](https://rustup.rs) — version is pinned to `1.95.0` via `rust-toolchain.toml`; `rustup` must be installed so `~/.cargo/bin` is on your PATH

## Project Structure

```
core/                         # Rust library (bitrufus_core)
BitRufus/                     # SwiftUI app source
  BitRufusApp.swift           #   App entry point
  ViewModels/                 #   Observable stores (AppStore, TorrentVM)
  Views/                      #   SwiftUI views (TorrentListView, AddMagnetSheet, FileSelectionSheet)
apps/TorrentApp/Generated/    # Auto-generated UniFFI Swift bindings (gitignored, rebuilt on each Xcode build)
scripts/build-rust.sh         # Build phase script: compiles Rust, stages .a, regenerates Swift bindings
BitRufusTests/                # XCTest unit tests
BitRufusUITests/              # XCTest UI tests
```

## Building

Open `BitRufus.xcodeproj` in Xcode and build the `BitRufus` scheme. The Run Script build phase automatically:
1. Compiles the Rust `core` crate for the active architecture
2. Stages `libbitrufus_core.a` to `target/active/` for Xcode to link
3. Regenerates `apps/TorrentApp/Generated/` (Swift bindings via UniFFI)

For Rust-only work:
```bash
cargo build --release -p bitrufus_core
cargo test -p bitrufus_core
cargo clippy --all-targets -- -D warnings
```

## Verifying the Setup

After a successful build and launch, the app shows an empty torrent list. Click `+` in the toolbar, paste a magnet link, and click Add. A file selection sheet will appear once the torrent's metadata resolves (typically a few seconds on a live network) — select the files you want and click Download. A row with the torrent's name, size, and live progress bar should appear in the list. Right-click any row to Pause, Resume, or Remove a torrent ("Remove and Delete Files" also erases downloaded data from `~/Downloads/TorrentApp/`).
