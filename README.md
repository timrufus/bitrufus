# BitRufus

A macOS torrent client with a Rust core and SwiftUI frontend, bridged via UniFFI.

## Prerequisites

- macOS 13.0+
- Xcode (latest stable)
- [Rust toolchain](https://rustup.rs) — version is pinned to `1.95.0` via `rust-toolchain.toml`; `rustup` must be installed so `~/.cargo/bin` is on your PATH

## Project Structure

```
core/                       # Rust library (bitrufus_core)
BitRufus/                   # SwiftUI app source (ContentView, app entry)
apps/TorrentApp/Generated/  # Auto-generated UniFFI Swift bindings (gitignored, rebuilt on each Xcode build)
scripts/build-rust.sh       # Build phase script: compiles Rust, stages .a, regenerates Swift bindings
BitRufusTests/              # XCTest unit tests
BitRufusUITests/            # XCTest UI tests
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

After a successful build, the app window should show a "Rust: pong" label at the bottom, confirming the Rust→Swift FFI roundtrip works.
