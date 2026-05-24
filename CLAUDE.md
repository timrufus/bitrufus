# BitRufus — AI Knowledge Base

## Build System

The project uses a two-layer build:
- Rust `core` crate → compiled to `libbitrufus_core.a` (staticlib) + `libbitrufus_core.dylib` (cdylib)
- Xcode links the `.a`; the `.dylib` is only used by `uniffi-bindgen` for introspection during binding generation

The Xcode Run Script build phase invokes `scripts/build-rust.sh`, which:
1. Compiles `core` for the active Xcode architecture (`aarch64-apple-darwin` or `x86_64-apple-darwin`)
2. Stages the static lib to `target/active/libbitrufus_core.a` — this fixed path is what Xcode links against
3. Runs `uniffi-bindgen` to regenerate `apps/TorrentApp/Generated/`

Both `staticlib` and `cdylib` in `core/Cargo.toml` `crate-type` are required — removing `cdylib` breaks the bindgen step.

## Generated Files

`apps/TorrentApp/Generated/` is gitignored and regenerated on every Xcode build. Do not edit or commit files in that directory:
- `bitrufus_core.swift` — Swift API bindings
- `bitrufus_coreFFI.h` — C header for the FFI layer
- `bitrufus_coreFFI.modulemap` — Swift module map

## Adding New Rust Functions to Swift

1. Add the function to `core/src/lib.rs` annotated with `#[uniffi::export]`
2. Build in Xcode (not just `cargo build`) — this triggers binding regeneration
3. The new function is then available in Swift

## Rust Toolchain

Pinned to `1.95.0` via `rust-toolchain.toml`. Do not change without verifying UniFFI 0.29 compatibility. `rustup` must be installed; the build script adds `~/.cargo/bin` to PATH (Xcode strips the shell PATH).

## Build Commands

```bash
# Rust only
cargo test -p bitrufus_core
cargo build --release -p bitrufus_core
cargo clippy --all-targets -- -D warnings

# Full app — requires Xcode
xcodebuild -project BitRufus.xcodeproj -scheme BitRufus -configuration Debug build
```

## Project Layout

- `core/` — Rust library crate (`bitrufus_core`)
- `BitRufus/` — SwiftUI app source (ContentView, app entry point)
- `apps/TorrentApp/Generated/` — generated Swift bindings (gitignored)
- `scripts/build-rust.sh` — Xcode build phase script
- `BitRufusTests/` — XCTest unit tests
- `BitRufusUITests/` — XCTest UI tests
