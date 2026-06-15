# Resource Efficiency: Idle CPU & Polling

## Overview
BitRufus is already lean on memory because of its stack (native SwiftUI + Rust/librqbit,
no Electron). No memory leaks were found. The avoidable cost is **idle CPU**: the stats
polling loop wakes the main thread and re-renders every row twice per second
unconditionally — even when the window is hidden, no torrents are active, or nothing has
changed. This plan removes that waste and reduces per-tick FFI overhead at scale, without
changing behavior the user can see.

Scope is ordered by impact. Tasks 1–2 remove almost all idle CPU and are low risk; Tasks
3–4 are scaling improvements; Task 5 is an investigation with uncertain payoff.

## Context (from discovery)
- `AppStore.init()` starts `statsPollingTask` — a Combine `Timer.publish(every: 0.5…)`
  loop calling `refreshStats()` ([AppStore.swift:78](../../BitRufus/ViewModels/AppStore.swift#L78)).
- `refreshStats()` loops all torrents and calls `engine.torrentStats(id:)` per torrent,
  then `vm.updateStats(s)` unconditionally ([AppStore.swift:221](../../BitRufus/ViewModels/AppStore.swift#L221)).
- `TorrentVM.updateStats` assigns to `@Published var stats` with no equality check
  ([AppStore.swift:17](../../BitRufus/ViewModels/AppStore.swift#L17) area).
- `engine.torrent_stats` is O(1): lock map, clone `Arc`, read in-memory `handle.stats()`
  ([engine.rs:217](../../core/src/engine.rs#L217)). Cheap per call, but called N times/tick.
- `pollMetadata` calls `engine.listTorrents()` inside its poll loop
  ([AppStore.swift:209](../../BitRufus/ViewModels/AppStore.swift#L209)); `list_torrents`
  rebuilds the **full** list with `handle.stats()` for every handle
  ([engine.rs:306](../../core/src/engine.rs#L306)).
- Runtime: `tokio = { features = ["rt-multi-thread", …] }`
  ([core/Cargo.toml](../../core/Cargo.toml)); the multi-thread pool sizes to the core count.
- No leaks: `statsPollingTask` captures `[weak self]` and is cancelled in `deinit`; other
  `Task {}` blocks are short-lived; `Engine` is `Arc`-held; handles cleaned under lock in
  `remove`.

## Findings (what's wrong, with evidence)

1. **Unconditional polling.** The 0.5 s timer wakes the main thread 2×/s for the whole app
   lifetime — regardless of window visibility, app activity, or whether any torrent is
   active. This defeats App Nap and keeps the CPU from idling. ([AppStore.swift:78](../../BitRufus/ViewModels/AppStore.swift#L78))

2. **Unconditional re-renders.** `updateStats` publishes on every tick even when the stats
   are identical (e.g. a paused torrent — same bytes, 0 speed). SwiftUI then re-renders
   every row 2×/s with no visible change. This is the single largest idle-CPU source.
   ([AppStore.swift:221](../../BitRufus/ViewModels/AppStore.swift#L221))

3. **N FFI crossings per tick.** One `torrentStats` call per torrent → for 100 torrents,
   200 FFI crossings + 200 lock acquisitions per second. Trivial for a few torrents, wasteful
   at scale.

4. **Quadratic metadata polling.** Each `pollMetadata` task calls `listTorrents()` (O(all
   torrents)) every 0.5 s; with several torrents resolving at once this is O(pending × all)
   work per tick. ([AppStore.swift:199](../../BitRufus/ViewModels/AppStore.swift#L199))

5. **Worker-thread pool size.** `rt-multi-thread` spawns one worker per core. Parked threads
   are cheap, but for this workload the pool can likely be capped. Lower priority; may be
   constrained by how UniFFI's tokio feature owns the runtime.

## Development Approach
- **Testing: Regular** (matches the existing `#[cfg(test)]` style in `engine.rs`).
- Rust changes get `cargo test -p bitrufus_core` unit tests.
- Swift/UI CPU changes have no XCTest harness here; verify with Xcode's CPU gauge / Activity
  Monitor / Instruments (Time Profiler) before-and-after (see Post-Completion).
- One task at a time; build green before the next. No visible behavior change.

## Implementation Steps

### Task 1: Publish stats only when they change

**Files:**
- Modify: `BitRufus/ViewModels/AppStore.swift`
- Modify: `core/src/types.rs` (only if `TorrentStats` is not already `Equatable`)

- [x] confirm the generated Swift `TorrentStats` conforms to `Equatable` (UniFFI records do
      by default); if not, add `#[derive(PartialEq)]` to the Rust `TorrentStats` record
- [x] in `TorrentVM.updateStats`, early-return when `newStats == stats` so `@Published`
      does not fire on unchanged data
- [x] verify paused/seeding rows stop re-rendering every tick (CPU gauge), while a live
      download still updates [manual test - skipped, not automatable]
- [x] no unit test (SwiftUI render count isn't unit-testable); covered by manual CPU check

### Task 2: Gate polling on app activity / visibility

**Files:**
- Modify: `BitRufus/ViewModels/AppStore.swift`
- Modify: `BitRufus/ContentView.swift` (or `BitRufusApp.swift`) to feed `scenePhase`

- [x] add `AppStore.setPolling(active:)` that starts the timer task when active and
      cancels it when inactive (recreate the `Task`; don't just no-op inside the loop, so
      the thread can truly idle)
- [x] drive it from `@Environment(\.scenePhase)` in the view: `.active` → on,
      `.inactive`/`.background` → off (macOS 13 supports `scenePhase` for `WindowGroup`)
- [x] optional: when active but **no** non-paused torrents exist, skip the FFI work (or use
      a slower cadence) — skipped; naive guard causes correctness regression after resume/pause without additional state tracking; main idle-CPU win already achieved by stopping the timer on background
- [x] handle edges: engine not ready yet (refreshStats already no-ops on nil engine);
      rapid active/inactive toggles must not spawn duplicate timer tasks (cancel the old
      task before starting a new one)
- [x] verify CPU drops to ~idle when the window is hidden/backgrounded, and resumes on focus [manual test - skipped, not automatable]

### Task 3: Batch stats into a single FFI call

**Files:**
- Modify: `core/src/engine.rs`
- Modify: `BitRufus/ViewModels/AppStore.swift`

- [x] extract the `rq.stats()`→`TorrentStats` mapping from `torrent_stats` into a private
      `fn stats_from_handle(id, &handle) -> TorrentStats`
- [x] add `pub fn all_stats(&self) -> Vec<TorrentStats>`: snapshot handles under the lock,
      release it, map each via the helper (mirror `list_torrents`' lock discipline)
- [x] rewrite `refreshStats()` to call `engine.allStats()` once, build an `[id: stats]`
      map, and update each `TorrentVM` (still respecting Task 1's equality guard)
- [x] write a Rust unit test: `all_stats()` returns one entry per managed handle
- [x] build (Xcode regenerates bindings) — `cargo test -p bitrufus_core` green

### Task 4: Poll a single torrent in `pollMetadata`

**Files:**
- Modify: `core/src/engine.rs`
- Modify: `BitRufus/ViewModels/AppStore.swift`

- [ ] add `pub fn torrent_info(&self, id: u64) -> Result<TorrentInfo, EngineError>` (O(1):
      lock, get handle, read name + `stats().total_bytes`)
- [ ] in `pollMetadata`, replace `engine.listTorrents().first(where:)` with
      `engine.torrentInfo(id:)`; on `NotFound` stop the loop (torrent was removed)
- [ ] write a Rust unit test: `torrent_info` returns the right name/size, and `NotFound`
      for an unknown id
- [ ] `cargo test -p bitrufus_core` green

### Task 5: (Investigate) cap tokio worker threads

**Files:**
- Investigate: `core/src/engine.rs`, `core/Cargo.toml`, UniFFI tokio integration

- [ ] determine whether the async FFI runtime is owned by UniFFI (tokio feature) or can be
      configured by us; check whether librqbit needs the multi-thread runtime
- [ ] if configurable, cap worker threads (e.g. `worker_threads(2–4)`) and measure thread
      count + CPU before/after
- [ ] if it's owned by UniFFI and not configurable without breaking async export, **stop**
      and record that here as a known constraint — do not fight the framework
- [ ] no behavior change; verify add/resolve/stats still work

### Task 6: Verify & document
- [ ] before/after CPU comparison: idle (window hidden), idle (paused torrents visible),
      one active download — record rough numbers
- [ ] confirm no functional regressions (add magnet, resolve, pause/resume, remove)
- [ ] run `cargo test -p bitrufus_core` and `cargo clippy --all-targets -- -D warnings`
- [ ] full Xcode build succeeds
- [ ] update CLAUDE.md "Concurrency Patterns" with the new polling model (gated timer,
      change-only publish, batched `all_stats`)
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion
*Manual / external — no checkboxes*

**Measurement (do this with the app running, which wasn't available at planning time):**
- Activity Monitor or Xcode Debug navigator → CPU gauge for steady-state CPU%.
- Instruments → Time Profiler to confirm the main-thread polling/render cost dropped.
- Compare RSS before/after (should be ~unchanged — this plan targets CPU, not memory).

**Notes / non-goals:**
- No memory leaks were found; this plan does not chase memory.
- Network/disk efficiency of librqbit itself is out of scope (it's already Rust/tokio).
- Task 5 may be a no-op if UniFFI owns the runtime — that's an acceptable outcome.
