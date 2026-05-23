# Plan: Foundation — Cargo Workspace and FFI Skeleton

## Overview

Set up the project skeleton: a Cargo workspace with a `core` crate that builds as a static library, an Xcode SwiftUI app under `apps/`, and a UniFFI-powered bridge between them. Deliverable is a one-call roundtrip — SwiftUI displays a string returned by a Rust function. No torrent logic in this plan; the goal is to lock in the build pipeline before any feature work touches it.

## Validation Commands

- `cargo build --release -p core`
- `cargo test -p core`
- `cargo clippy --all-targets -- -D warnings`
- `xcodebuild -project apps/TorrentApp.xcodeproj -scheme TorrentApp -configuration Debug build`
- Manual: open `apps/TorrentApp.xcodeproj`, press ⌘R, confirm app window displays "pong".

### Task 1: Create Cargo workspace and core crate

- [x] Initialize root `Cargo.toml` declaring a single workspace member `core`.
- [x] Create `core/Cargo.toml` with `crate-type = ["staticlib", "cdylib"]` and minimal deps (`uniffi`, `tokio` with `rt-multi-thread`, `thiserror`).
- [x] Add a `[[bin]] uniffi-bindgen` entry pointing to `core/uniffi-bindgen.rs`.
- [x] Verify `cargo build` succeeds and produces `target/release/libcore.a`.

### Task 2: Wire up UniFFI scaffolding

- [x] Add `uniffi::setup_scaffolding!()` to `core/src/lib.rs`.
- [x] Implement `core/uniffi-bindgen.rs` calling `uniffi::uniffi_bindgen_main()`.
- [x] Export one trivial function: `#[uniffi::export] pub fn ping() -> String`.
- [x] Generate Swift bindings via the bindgen binary against the built staticlib and confirm a `torrent_core.swift` plus module map are produced.

### Task 3: Create Xcode SwiftUI app

- [x] Create `apps/TorrentApp.xcodeproj` via Xcode (macOS App, SwiftUI lifecycle, minimum macOS 14.0).
- [x] Keep `TorrentAppApp.swift` and `ContentView.swift`; strip the stock placeholder content.
- [x] Set bundle identifier and signing to ad-hoc / development; no team needed.

### Task 4: Integrate Rust build into Xcode

- [x] Write `scripts/build-rust.sh` that builds the core crate for the active Xcode arch and regenerates Swift bindings into `apps/TorrentApp/Generated/`.
- [x] Add a Run Script Build Phase invoking `scripts/build-rust.sh` before "Compile Sources".
- [x] Link `libcore.a` under "Link Binary With Libraries"; add `apps/TorrentApp/Generated/` to the app target's compile sources.
- [x] Update `ContentView.swift` to display `Text(ping())`.

### Task 5: Lock down the toolchain and ignore generated artifacts

- [ ] Add `rust-toolchain.toml` pinning a stable Rust version.
- [ ] Add `.gitignore` covering `target/`, `build/`, `.DS_Store`, `apps/TorrentApp/Generated/`, `xcuserdata/`, `*.xcuserstate`.
- [ ] Run the full validation command set and confirm a fresh checkout builds end-to-end.
