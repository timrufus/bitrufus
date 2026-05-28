# BitRufus — AI Knowledge Base

## Build System

The project uses a two-layer build:
- Rust `core` crate → compiled to `libbitrufus_core.a` (staticlib) + `libbitrufus_core.dylib` (cdylib)
- Xcode links the `.a`; the `.dylib` is only used by `uniffi-bindgen` for introspection during binding generation

The Xcode Run Script build phase invokes `scripts/build-rust.sh`, which:
1. Resolves the target arch (handles Xcode's `CURRENT_ARCH=undefined_arch` for non-per-arch script phases by falling back to `ARCHS` or `uname -m`)
2. Compiles `core` for that arch (`aarch64-apple-darwin` or `x86_64-apple-darwin`)
3. Stages the static lib to `target/active/libbitrufus_core.a` — this fixed path is what Xcode links against
4. Runs `uniffi-bindgen` to regenerate `apps/TorrentApp/Generated/`

Both `staticlib` and `cdylib` in `core/Cargo.toml` `crate-type` are required — removing `cdylib` breaks the bindgen step.

## Deployment Target

macOS 13.0. Set in `BitRufus.xcodeproj/project.pbxproj` as `MACOSX_DEPLOYMENT_TARGET = 13.0;` (four occurrences — keep them in sync). Don't raise without a concrete API need; Xcode 14.2 ships with the macOS 13 SDK and cannot target 14.x.

## Generated Files

`apps/TorrentApp/Generated/` is gitignored and regenerated on every Xcode build. Do not edit or commit files in that directory:
- `bitrufus_core.swift` — Swift API bindings
- `bitrufus_coreFFI.h` — C header for the FFI layer
- `bitrufus_coreFFI.modulemap` — Swift module map

## Adding New Rust Functions to Swift

Free functions: add to `core/src/lib.rs` annotated with `#[uniffi::export]`, then build in Xcode.

Object types (classes in Swift): derive `uniffi::Object` on the struct, put methods in `#[uniffi::export] impl MyType { ... }`, and use `#[uniffi::constructor]` for constructors (which must return `Arc<Self>`). Async constructors and methods are supported. After any change, build in Xcode (not just `cargo build`) to regenerate the Swift bindings.

## Session Persistence

The Engine stores torrent state as JSON in `~/Library/Application Support/BitRufus/BitRufus/` (resolved via the `directories` crate at runtime). Deleting this directory resets all persisted torrent state. This is the first place to look when debugging "torrent reappears after restart" or "session not loading" issues.

Downloaded torrent files land in `~/Downloads/TorrentApp/` — this path is set by `AppStore.startEngine()` in `BitRufus/ViewModels/AppStore.swift` and passed to `Engine(downloadDir:)`. It is distinct from the JSON session persistence path above.

Engine IDs are stable across restarts: `Engine::new` maps each restored librqbit session ID `sid` to engine ID `sid + 1`. The same torrent always gets the same engine ID across restarts, even after other torrents have been removed. This is the first place to look when debugging "torrent ID changed after restart" issues.

## Concurrency Patterns

**Concurrent add/remove safety (`deleting` tombstone set):** `EngineInner` carries a `deleting: HashSet<usize>` alongside `handles`. When `remove()` is called, the librqbit session ID is inserted into `deleting` and the handle is removed from `handles` — both atomically under the same lock — before `session.delete()` is awaited. Any new Engine method that calls `session.add_torrent` and may receive an `AlreadyManaged` response must check `inner.deleting` before inserting into `handles`; otherwise a concurrent remove will leave a zombie handle after the delete completes. See `add_magnet` in `core/src/engine.rs` for the reference implementation.

**Async engine initialization in AppStore:** `Engine(downloadDir:)` is an async constructor. `AppStore.init()` fires a detached `Task { await startEngine() }` and exposes `@Published var isEngineReady: Bool` (set to `true` after the engine is ready). UI elements that require a live engine gate on `isEngineReady` (e.g., the `+` toolbar button in `TorrentListView`). Do not call engine methods synchronously from `AppStore.init()`.

**Two-phase magnet add flow:** `AppStore.addMagnet(_:)` adds the torrent to the engine in a paused state and returns a `TorrentVM?` but does NOT append it to `torrents`. The caller (`TorrentListView`) must present `FileSelectionSheet` and then call either `confirmTorrent(_:)` (after `setFileSelection`) or `cancelTorrent(_:)` (calls `engine.remove(deleteFiles: true)` and discards the VM). A torrent that is neither confirmed nor cancelled remains live in the engine session until the next restart. `addMagnet` returns `nil` when the torrent ID is already present in `torrents` (duplicate magnet re-add after confirmation is a no-op).

**`torrentFiles` before metadata resolves:** `engine.torrentFiles(id:)` calls `handle.with_metadata(...)` internally, which returns an error when torrent metadata has not yet been fetched from DHT/trackers. `AppStore.torrentFiles(id:)` silently converts this error to `[]`. Always use `AppStore.waitForTorrentFiles(id:)` when fetching files to show to the user — it polls up to 30 s for metadata to resolve before returning.

**Stats polling (`statsPollingTask`):** `AppStore.init()` starts a `Task` that fires a Combine timer every 500 ms and calls `refreshStats()`. The task starts before the engine is ready; `refreshStats()` is a no-op until `engine` is non-nil. The task is cancelled in `deinit` via `statsPollingTask?.cancel()`. `TorrentVM.stats` is `nil` until the first successful poll after the engine is ready (typically within 500 ms of `isEngineReady` becoming `true`).

**Background metadata polling (`pollMetadata`):** Called for any torrent with `totalBytes == 0` — both newly-confirmed magnets (from `confirmTorrent`) and restored torrents (from `startEngine`). Uses a two-phase backoff: 0.5 s × 60 polls (30 s window), then 5 s × 60 polls (5 min window). On resolution it calls `vm.refreshInfo(_:)`, which updates the VM's name and size while preserving any existing display name. Distinct from `waitForTorrentFiles` — that blocks the caller; `pollMetadata` runs as a background task and updates the VM in place.

**Pause/Resume/Remove from `TorrentRow`:** `TorrentRow` exposes a context menu driven by `vm.stats?.state`. "Pause" and "Resume" are mutually exclusive based on the current state. "Remove…" presents a `confirmationDialog` offering "Remove" (`deleteFiles: false` — keeps downloaded data in `~/Downloads/TorrentApp/`) or "Remove and Delete Files" (`deleteFiles: true`). `AppStore.remove(id:deleteFiles:)` awaits `engine.remove`, then removes the VM from `torrents` on success; on failure the engine reinstates the handle so both layers stay consistent.

## Rust Toolchain

Pinned to `1.95.0` via `rust-toolchain.toml`. Do not change without verifying UniFFI 0.29 compatibility. `rustup` must be installed; the build script adds `~/.cargo/bin` to PATH (Xcode strips the shell PATH).

## Core Dependencies

- `uniffi = "0.29"` (features: `build`, `cli`, `tokio`) — FFI binding generator
- `tokio = "1"` (features: `rt-multi-thread`, `macros`)
- `thiserror = "2"`
- `librqbit = "8.1.1"` — torrent backend (Session, ManagedTorrent, etc.); version is load-bearing for API compatibility
- `directories = "6"` — resolves the macOS Application Support path for JSON session persistence
- `tempfile` (dev-dependency) — creates temporary directories in tests; not linked into the final binary

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
- `BitRufus/` — SwiftUI app source (`BitRufusApp.swift` entry point, `ContentView.swift`, `ViewModels/`, `Views/`)
- `apps/TorrentApp/Generated/` — generated Swift bindings (gitignored)
- `scripts/build-rust.sh` — Xcode build phase script
- `BitRufusTests/` — XCTest unit tests
- `BitRufusUITests/` — XCTest UI tests
- `docs/` — design notes and plans

---

## Behavioral Guidelines

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

### 1. Think Before Coding

Don't assume. Don't hide confusion. Surface tradeoffs.

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them — don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

### 2. Simplicity First

Minimum code that solves the problem. Nothing speculative.
- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

### 3. Surgical Changes

Touch only what you must. Clean up only your own mess.

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it — don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

### 4. Goal-Driven Execution

Define success criteria. Loop until verified.

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

These guidelines are working if: fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
