# Add .torrent File Support

## Overview
BitRufus currently ingests torrents only via magnet links (`Engine.add_magnet` →
`AppStore.addMagnet` → `AddMagnetSheet`). This plan adds support for `.torrent`
files through two new entry points:

1. **File picker** — `NSOpenPanel` filtered to `.torrent`, invoked from a toolbar/menu action.
2. **Drag & drop** — `.onDrop` on the torrent list accepting both `.torrent` file URLs
   and magnet text.

Both entry points funnel into the **existing** two-phase add flow
(`pendingVM` → `FileSelectionSheet` → `confirmTorrent`/`cancelTorrent`), so no new
add/confirm machinery is introduced.

**Key benefit over magnet:** `.torrent` files embed the full `info` dictionary
(name, size, piece hashes, file list). Metadata is available the instant the torrent
is added, so `waitForTorrentFiles` returns immediately and the `pollMetadata`
background loop is unnecessary for these torrents.

**Out of scope** (separate plan): the loading/empty/list empty-state redesign and a
styled drop-zone surface. This plan attaches `.onDrop` to the current `List` with a
lightweight drag-over highlight only.

## Context (from discovery)
- **Rust core** (`core/src/engine.rs`): `add_magnet` (lines 95–171) parses a `Magnet`,
  rejects BTv2-only links via `parsed.as_id20()`, calls `session.add_torrent(AddTorrent::from_url(...))`,
  then runs a lock-guarded block (lines 140–159) that checks the `deleting` tombstone set,
  dedups handles via `Arc::ptr_eq`, and allocates an engine ID. This concurrency-critical
  block is identical to what a `.torrent` add needs.
- **librqbit 8.1.1** (`session.rs`): `AddTorrent::from_bytes(impl Into<Bytes>)` exists
  (line 347); `Vec<u8>` converts to `Bytes` directly. `AddTorrentResponse` variants
  (`Added` / `AlreadyManaged` / `ListOnly`) are the same for file and URL adds.
- **Errors** (`core/src/types.rs`): `EngineError` has `InvalidMagnet`, `NotFound`,
  `Io`, `Backend`. No variant for a malformed torrent file.
- **Swift** (`BitRufus/ViewModels/AppStore.swift`): `addMagnet` (lines 87–92) adds to the
  engine, returns `nil` on duplicate, and the caller drives confirm/cancel.
  `waitForTorrentFiles` (lines 119–126) polls up to 30 s; `pollMetadata` (lines 130–150)
  is the magnet-only background resolver.
- **Swift** (`BitRufus/Views/TorrentListView.swift`): the two-phase flow lives in the
  `AddMagnetSheet`'s `.sheet(onDismiss:)` closure (lines 42–64) — it calls
  `waitForTorrentFiles`, then either presents `FileSelectionSheet` or cancels.
  `AddMagnetSheet` (`Views/AddMagnetSheet.swift`) reports the duplicate case as an alert.
- **Sandbox** (`BitRufus/BitRufus.entitlements`): `com.apple.security.files.user-selected.read-write`
  is **already present** (line 13) — no entitlement change needed.
- **UniFFI**: `Engine` is a `uniffi::Object`; methods live in a `#[uniffi::export] impl Engine`
  block. New methods are added there and bindings regenerate on Xcode build.

## Development Approach
- **Testing approach: TDD (tests first)** — matches the existing `#[cfg(test)] mod tests`
  in `engine.rs`. Write the failing Rust test, then implement.
- Complete each task fully (impl + tests green) before the next.
- Small, focused changes; match existing style (error mapping helpers, lock discipline).
- **Every task includes tests.** Rust: `cargo test -p bitrufus_core`. Swift view-layer
  changes are validated by an Xcode build + manual smoke (no XCTest harness for SwiftUI
  drop/picker exists today — see Post-Completion).
- Maintain backward compatibility: `add_magnet` behavior must not change.

## Testing Strategy
- **Unit tests (Rust)**: required per task. Cover success (valid `.torrent` bytes →
  correct id/name/size), and error/edge cases (corrupt bytes, duplicate add, add-during-delete).
  Use the `tempfile` dev-dependency for the download dir, as existing tests do.
- **Test fixture**: a small valid single-file `.torrent` is needed. Generate one in-test
  by bencoding a minimal `info` dict (no trackers required for an add+parse test), or
  check in a tiny fixture under `core/tests/fixtures/`. Prefer in-test generation to avoid
  binary blobs in the repo (decide in Task 1).
- **e2e**: project has no SwiftUI UI-test harness for drop/picker; the picker and
  `.onDrop` paths are verified by Xcode build + manual smoke test (Post-Completion).

## Progress Tracking
- mark completed items `[x]` immediately
- new tasks: ➕ prefix; blockers: ⚠️ prefix
- keep this file in sync if scope shifts

## Solution Overview
- **Rust**: extract the shared post-add concurrency block from `add_magnet` into a private
  helper `register_added_handle(&self, response, fallback_name) -> Result<TorrentInfo, EngineError>`,
  then add `add_torrent_file(&self, bytes: Vec<u8>)` that calls
  `session.add_torrent(AddTorrent::from_bytes(bytes), paused)` and reuses the helper.
  This keeps the load-bearing `deleting`/`ptr_eq`/ID-allocation logic in ONE place
  (per CLAUDE.md's zombie-handle warning, duplicating it is risky).
- **New error variant** `EngineError::InvalidTorrent { reason }` for unparseable/corrupt
  bytes, mirroring `InvalidMagnet`. Update Swift `engineErrorMessage`.
- **Swift**: `AppStore.addTorrentFile(data: Data) async throws -> TorrentVM?` mirroring
  `addMagnet` (duplicate → `nil`). No `pollMetadata` call needed (size known at add time;
  `confirmTorrent` already guards `vm.info.totalBytes == 0`, so it naturally skips polling).
- **Swift view**: extract the inline file-selection logic from `AddMagnetSheet.onDismiss`
  into a reusable `TorrentListView.beginFileSelection(for: TorrentVM)`; call it from the
  sheet dismiss, the picker, and the drop handler.

### Key design decisions
1. **Shared Rust helper vs. duplicate** — chosen: shared helper. The concurrency block is
   subtle and safety-critical; a single implementation prevents drift. Trade-off: `add_magnet`
   is lightly refactored (acceptable — covered by existing tests).
2. **BTv2-only handling** — `add_magnet` pre-rejects v2 via `as_id20()`. For a `.torrent`
   file we cannot cheaply pre-check without parsing the bencode ourselves. Decision: let
   librqbit parse; if it rejects or fails, surface as `InvalidTorrent`/`Backend`. Hybrid
   (v1+v2) torrents work via their v1 infohash. Document pure-v2 as a known limitation
   rather than adding a bespoke parser.
3. **Name extraction** — for `.torrent`, `handle.name()` resolves immediately from the
   embedded `info` dict; the `dn=` fallback used in `add_magnet` does not apply. Pass an
   empty fallback name to the helper.

## Technical Details
- `AddTorrent::from_bytes(bytes)` takes `impl Into<Bytes>`; `Vec<u8>: Into<Bytes>` holds.
- Add options identical to magnet: `AddTorrentOptions { paused: true, ..Default::default() }`.
- `register_added_handle` signature handles all three `AddTorrentResponse` arms exactly as
  `add_magnet` does today (reject `ListOnly`).
- Swift picker: `NSOpenPanel` with `allowedContentTypes = [UTType(filenameExtension: "torrent")].compactMap { $0 }`
  (fallback to `[.data]` if nil); `canChooseFiles = true`, `allowsMultipleSelection = false`
  (single file for v1; multi-select is a possible follow-up). Read bytes with
  `Data(contentsOf: url)` inside the panel's granted scope.
- Swift drop: `.onDrop(of: [.fileURL, .text], isTargeted: $isDropTargeted)`. For `.fileURL`,
  load the URL, accept only `.torrent` extension, read `Data`, call `addTorrentFile`. For
  `.text`/`.url`, treat as magnet → existing `addMagnet`. Ignore everything else.

## What Goes Where
- **Implementation Steps** (checkboxes): Rust helper + method + error variant + tests;
  Swift store method; Swift picker; Swift drop; view refactor; binding regen via build.
- **Post-Completion** (no checkboxes): manual smoke tests of picker/drop in the running
  app, pure-v2 limitation note, multi-file picker follow-up.

## Implementation Steps

### Task 1: Add `InvalidTorrent` error variant

**Files:**
- Modify: `core/src/types.rs`
- Modify: `core/src/engine.rs` (test module)

- [ ] add `#[error("invalid torrent file: {reason}")] InvalidTorrent { reason: String }` to `EngineError`
- [ ] decide test-fixture strategy (in-test bencode generation vs. checked-in fixture) and document it in this file
- [ ] write a failing test asserting corrupt bytes (e.g. `b"not a torrent"`) → `EngineError::InvalidTorrent` (test references the not-yet-added method; mark `[x] ... (fails until Task 2)`)
- [ ] run `cargo test -p bitrufus_core` — confirms variant compiles

### Task 2: Extract `register_added_handle` and add `add_torrent_file` (Rust)

**Files:**
- Modify: `core/src/engine.rs`

- [ ] extract lines ~124–170 of `add_magnet` (handle resolution, `deleting` check, `ptr_eq` dedup, ID allocation, `TorrentInfo` build) into private `fn register_added_handle(&self, response: AddTorrentResponse, fallback_name: Option<String>) -> Result<TorrentInfo, EngineError>`
- [ ] rewrite `add_magnet` to call `register_added_handle(response, dn)` — behavior unchanged
- [ ] add `pub async fn add_torrent_file(&self, bytes: Vec<u8>) -> Result<TorrentInfo, EngineError>` inside the `#[uniffi::export] impl Engine` block: call `session.add_torrent(AddTorrent::from_bytes(bytes), paused opts)`, map parse/add errors to `InvalidTorrent` (corrupt) / `Backend` (other), then `register_added_handle(response, None)`
- [ ] write test: valid `.torrent` bytes → `Ok(TorrentInfo)` with non-empty `name` and `total_bytes > 0` (immediate metadata)
- [ ] write test: duplicate add (same bytes twice) → `AlreadyManaged` path returns the **same** engine id (mirror the magnet dedup test)
- [ ] write test: add-during-delete safety — confirm `deleting` check still applies via the shared helper (extend/mirror existing concurrency test if feasible; otherwise note coverage via `add_magnet`'s existing test since the path is shared)
- [ ] flip the Task 1 corrupt-bytes test to passing
- [ ] run `cargo test -p bitrufus_core` and `cargo clippy --all-targets -- -D warnings` — must pass before next task

### Task 3: Regenerate bindings + `AppStore.addTorrentFile` (Swift)

**Files:**
- Modify: `BitRufus/ViewModels/AppStore.swift`

- [ ] build in Xcode (`xcodebuild ... build`) to regenerate `apps/TorrentApp/Generated/` with `addTorrentFile`
- [ ] add `func addTorrentFile(_ data: Data) async throws -> TorrentVM?` mirroring `addMagnet`: call `engine.addTorrentFile(bytes:)`, return `nil` if `torrents` already contains the id, else return a new `TorrentVM(info:)`
- [ ] update `engineErrorMessage` to handle `EngineError.InvalidTorrent(let reason)`
- [ ] confirm no `pollMetadata` is wired for this path (size is known; `confirmTorrent`'s `totalBytes == 0` guard already skips it)
- [ ] build — must succeed before next task (no Rust unit test applies to this Swift wrapper; covered by manual smoke in Post-Completion)

### Task 4: Refactor shared file-selection entry in TorrentListView

**Files:**
- Modify: `BitRufus/Views/TorrentListView.swift`

- [ ] extract the `waitForTorrentFiles` → `fileSelectionItem`/`cancelTorrent` logic from the `AddMagnetSheet` `.sheet(onDismiss:)` closure (lines ~42–64) into `private func beginFileSelection(for vm: TorrentVM)`
- [ ] call `beginFileSelection(for:)` from the existing sheet `onDismiss` (behavior unchanged for magnet)
- [ ] build and manually verify magnet add still works end-to-end (regression guard)
- [ ] build — must succeed before next task

### Task 5: Add `.torrent` file picker

**Files:**
- Modify: `BitRufus/Views/TorrentListView.swift`

- [ ] add a toolbar button (or menu item next to `+`) "Open Torrent File…" gated on `store.isEngineReady && pendingVM == nil`
- [ ] present `NSOpenPanel` (single file, `.torrent` content type, fallback `.data`); on selection read `Data(contentsOf:)` inside the granted scope
- [ ] on read, call `store.addTorrentFile(_:)`; on non-nil VM set `pendingVM` and call `beginFileSelection(for:)`; on `nil` show "already in the list" alert; on throw show `engineErrorMessage`
- [ ] handle edge: file unreadable/deleted between pick and read → surface error via `actionError`
- [ ] build and manually verify picker adds a `.torrent` and reaches `FileSelectionSheet` (manual; logged in Post-Completion)

### Task 6: Add drag & drop (.torrent files + magnet text)

**Files:**
- Modify: `BitRufus/Views/TorrentListView.swift`

- [ ] add `@State private var isDropTargeted = false` and `.onDrop(of: [.fileURL, .text], isTargeted: $isDropTargeted)` on the list
- [ ] in the drop handler: for a `.fileURL` provider, load the URL, accept only `.torrent` extension, read `Data`, route through `addTorrentFile` + `beginFileSelection`
- [ ] for `.text`/`.url` providers, trim and route through existing `addMagnet` + `beginFileSelection`
- [ ] ignore unsupported types and non-`.torrent` files silently (return `false`); on multiple items, process the first valid one (multi-add is a follow-up)
- [ ] add a lightweight drag-over highlight bound to `isDropTargeted` (border/overlay only — NOT the full empty-state redesign)
- [ ] build and manually verify: drop a `.torrent` file, drop magnet text, drop an unsupported file (no crash, ignored) — logged in Post-Completion

### Task 7: Verify acceptance criteria
- [ ] valid `.torrent` via picker and via drop both reach `FileSelectionSheet` and confirm into the list with correct name/size
- [ ] corrupt `.torrent` shows a clear `InvalidTorrent` error, no crash
- [ ] duplicate `.torrent` reports "already in the list"
- [ ] magnet add (sheet + dropped text) still works (regression)
- [ ] run full Rust suite: `cargo test -p bitrufus_core`
- [ ] run `cargo clippy --all-targets -- -D warnings`
- [ ] full Xcode build succeeds: `xcodebuild -project BitRufus.xcodeproj -scheme BitRufus -configuration Debug build`

### Task 8: [Final] Update documentation
- [ ] update CLAUDE.md: note `.torrent` ingestion (`add_torrent_file`, `register_added_handle` shared helper, picker/drop entry points, immediate-metadata path that skips `pollMetadata`)
- [ ] update README.md if it lists features/usage
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion
*Items requiring manual intervention or external systems — informational only*

**Manual verification:**
- Run the app and smoke-test all three drop cases (`.torrent` file, magnet text, unsupported file) and the picker — SwiftUI drop/picker have no XCTest coverage in this project.
- Verify sandbox actually permits reading a `.torrent` from outside `~/Downloads` (e.g. `~/Desktop`) — entitlement is present but confirm at runtime.
- Confirm a hybrid (v1+v2) `.torrent` adds via its v1 infohash.

**Known limitations / follow-ups:**
- Pure BTv2-only `.torrent` files are not pre-validated (no `as_id20` equivalent for files); they rely on librqbit and may fail with a backend error. Document as a known limitation.
- Multi-file picker selection and multi-item drop are deferred (single item per add for now).
- Empty-state redesign + styled drop-zone surface is a **separate plan** (user is doing the magnet-only empty state first).
