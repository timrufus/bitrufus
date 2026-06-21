# BitRufus

A macOS torrent client with a Rust core and SwiftUI frontend, bridged via UniFFI.

## Screenshots

![Main window](docs/screenshots/main-window.png)

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| macOS | 13.0+ | Ventura or later |
| Xcode | 14.2+ | Includes the macOS 13 SDK |
| Rust / rustup | any | Pinned to `1.95.0` via `rust-toolchain.toml`; rustup downloads it automatically |

## Project Structure

```
core/                         # Rust library crate (bitrufus_core)
BitRufus/                     # SwiftUI app source
  BitRufusApp.swift           #   App entry point and menu commands
  ContentView.swift           #   Root view
  ViewModels/
    AppStore.swift            #   Central observable store (engine lifecycle, torrent list, stats polling)
  Views/
    TorrentListView.swift     #   Main torrent list with toolbar
    AddMagnetSheet.swift      #   Paste-magnet sheet
    FileSelectionSheet.swift  #   Per-file selection before download starts
    SettingsView.swift        #   Download directory picker
    DiskSpacePopover.swift    #   Disk space indicator popover
  Persistence/
    TorrentStore.swift        #   Display-name side-file (torrents.json)
    AppSettings.swift         #   User preferences (download directory)
  Assets.xcassets             #   App icons and colors
apps/TorrentApp/Generated/    # Auto-generated UniFFI Swift bindings (gitignored, rebuilt on each Xcode build)
scripts/build-rust.sh         # Xcode build phase: compiles Rust, stages .a, regenerates Swift bindings
BitRufusTests/                # XCTest unit tests
BitRufusUITests/              # XCTest UI tests
docs/                         # Screenshots and design plans
```

## Building from Source

### 1. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Follow the on-screen prompts (default install is fine)
source "$HOME/.cargo/env"
```

The project pins to Rust `1.95.0`. When you first build in Xcode, `rustup` automatically downloads this toolchain if it isn't already installed.

### 2. Add the required targets

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

### 3. Clone and open in Xcode

```bash
git clone <repo-url>
cd BitRufus
open BitRufus.xcodeproj
```

### 4. Build

Select the **BitRufus** scheme, choose your Mac as the run destination, and press **⌘B** (or **⌘R** to build and run).

The Xcode Run Script build phase automatically:
1. Compiles the Rust `core` crate for the active architecture (`aarch64` or `x86_64`)
2. Stages `libbitrufus_core.a` to `target/active/` for Xcode to link
3. Regenerates `apps/TorrentApp/Generated/` (Swift bindings via UniFFI)

> **Note:** The first build downloads crates and may take a few minutes. Subsequent builds are incremental.

### Rust-only workflow

```bash
cargo build -p bitrufus_core
cargo test -p bitrufus_core
cargo clippy --all-targets -- -D warnings
```

> Running `cargo build` outside Xcode does **not** regenerate the Swift bindings. Use Xcode (or `xcodebuild`) when you change the public Rust API.

### Command-line build (CI / no GUI)

```bash
xcodebuild -project BitRufus.xcodeproj -scheme BitRufus -configuration Debug build
```

## Verifying the Setup

After a successful build and launch:

1. The app shows an empty torrent list.
2. Add a torrent via any of these methods:
   - Click **+** in the toolbar and paste a magnet link, then click **Add**.
   - Click **Open Torrent File…** in the toolbar and pick a `.torrent` file.
   - Drag a `.torrent` file (or paste magnet text) onto the torrent list.
3. A file-selection sheet appears (immediately for `.torrent` files; after a few seconds for magnet links while metadata resolves). Select the files you want and click **Download**.
4. A row with the torrent name, size, and a live progress bar appears in the list.
5. Right-click any row to **Pause**, **Resume**, or **Remove** a torrent. "Remove and Delete Files" also erases downloaded data.

## Download Location

By default, files are saved to `~/Downloads/TorrentApp/`. You can change this in **BitRufus → Settings…** (⌘,) — the folder picker lets you choose any directory.

Torrent session state (survives restarts) is stored separately in `~/Library/Application Support/com.BitRufus.BitRufus/`.

## Known Limitations

- No bandwidth throttling (upload or download limits).
- No sequential download mode.
- No system notifications or dock badge.
- No code signing or notarization — the binary runs only on the machine that built it.
- Pure BitTorrent v2-only `.torrent` files are not pre-validated and may fail with a backend error; hybrid v1+v2 torrents work via their v1 infohash.

## Roadmap

Features intentionally out of scope for the current version:

- Bandwidth limits (upload and download throttling)
- Sequential download mode
- System notifications on completion
- Dock badge showing active download count
- Code signing and notarization
- RSS feed support and auto-download rules
- Scheduling (start/stop at specific times)

## Troubleshooting

**"module 'bitrufus_core' not found" or type mismatch errors**

The Swift bindings in `apps/TorrentApp/Generated/` are regenerated on every Xcode build. If you see Swift errors referencing generated types, do a clean build: **Product → Clean Build Folder** (⇧⌘K), then build again.

**Architecture mismatch ("building for macOS-arm64 but attempting to link with file built for macOS-x86_64")**

`scripts/build-rust.sh` resolves the arch from `$CURRENT_ARCH`, falling back to `$ARCHS` and then `uname -m`. If you see an arch mismatch, confirm that the Xcode scheme's Build Settings are not overriding `ARCHS` to a cross-target value. Rosetta builds are not supported.

**`cargo` not found during build**

Xcode strips the login-shell PATH. The build script explicitly prepends `~/.cargo/bin`. If the build phase fails with "command not found: cargo", verify that `rustup` is installed for your user account (`which cargo` in Terminal should print `~/.cargo/bin/cargo`). Re-run the installer if needed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Wrong Rust toolchain version**

`rust-toolchain.toml` pins to `1.95.0`. If `rustup` is installed but the pinned toolchain is missing, the build script triggers `rustup` to download it automatically. If that fails (air-gapped environment, proxy), install manually:

```bash
rustup toolchain install 1.95.0
```
