import SwiftUI
import AppKit
import UniformTypeIdentifiers

struct FileSelectionItem: Identifiable {
    let vm: TorrentVM
    let files: [FileInfo]
    var id: UInt64 { vm.id }
}

struct TorrentListView: View {
    @EnvironmentObject private var store: AppStore
    @State private var showAddSheet = false
    @State private var fileSelectionItem: FileSelectionItem?
    @State private var actionError: String?
    @State private var showDiskSpace = false
    @State private var isDropTargeted = false
    @State private var isHandlingFileDrop = false

    var body: some View {
        contentArea
            .onDrop(of: [.fileURL, .url, .text], isTargeted: $isDropTargeted) { providers in
                handleDrop(providers: providers)
            }
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(Color.accentColor.opacity(isDropTargeted ? 1 : 0), lineWidth: 2)
                    .padding(1)
            )
            .onChange(of: store.autoPresentSelectionFor) { newValue in
                guard let id = newValue else { return }
                store.autoPresentSelectionFor = nil
                presentSelection(forID: id)
            }
            .toolbar { mainToolbar }
            .sheet(isPresented: $showAddSheet) { AddMagnetSheet() }
            .background(fileSelectionSheetHost)
            .alert("Engine Error", isPresented: engineErrorBinding) {
                Button("OK", role: .cancel) { store.clearEngineError() }
            } message: {
                Text(store.engineStartError ?? "")
            }
            .alert("Error", isPresented: actionErrorBinding) {
                Button("OK", role: .cancel) { actionError = nil }
            } message: {
                Text(actionError ?? "")
            }
            .toolbarBackground(
                LinearGradient(
                    colors: [Color.blue.opacity(0.18), Color.indigo.opacity(0.22)],
                    startPoint: .leading,
                    endPoint: .trailing
                ),
                for: .windowToolbar
            )
            .frame(minWidth: 500, minHeight: 300)
    }

    private var contentArea: some View {
        Group {
            if store.torrents.isEmpty && store.pendingMagnets.isEmpty {
                emptyState
            } else {
                List {
                    ForEach(store.pendingMagnets) { pending in
                        PendingMagnetRow(
                            pending: pending,
                            onCancel: { store.cancelPending(pending) },
                            onRetry: { store.retryPending(pending) }
                        )
                    }
                    ForEach(store.torrents) { vm in
                        TorrentRow(vm: vm, onChooseFiles: { presentSelection(forID: vm.id) })
                    }
                }
                .scrollContentBackground(.hidden)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
    }

    @ToolbarContentBuilder
    private var mainToolbar: some ToolbarContent {
        ToolbarItem(placement: .automatic) {
            Button { showDiskSpace.toggle() } label: {
                Label("Disk Space", systemImage: "internaldrive")
            }
            .popover(isPresented: $showDiskSpace) { DiskSpacePopover() }
        }
        ToolbarItem {
            Button { showAddSheet = true } label: {
                Label("Add Torrent", systemImage: "plus")
            }
            .disabled(!store.isEngineReady)
        }
        ToolbarItem {
            Button { openTorrentFilePicker() } label: {
                Label("Open Torrent File\u{2026}", systemImage: "doc.badge.plus")
            }
            .disabled(!store.isEngineReady || fileSelectionItem != nil || isHandlingFileDrop)
        }
    }

    // Attach the file-selection sheet to a background EmptyView so both sheets
    // coexist on macOS 13.0–13.2 (only one .sheet per view was supported before
    // macOS 13.3; attaching to a separate view node avoids the conflict).
    private var fileSelectionSheetHost: some View {
        EmptyView()
            // Plain dismiss (Esc / click-away) leaves the torrent in the
            // awaiting-files state so it can be reopened from its row; only the
            // explicit Download / Cancel buttons below act on it.
            .sheet(item: $fileSelectionItem) { item in
                FileSelectionSheet(
                    vm: item.vm,
                    files: item.files,
                    onConfirm: { selectedIndexes in
                        let id = item.vm.id
                        fileSelectionItem = nil
                        Task {
                            do { try await store.applyFileSelection(id: id, selectedIndexes: selectedIndexes) }
                            catch { actionError = engineErrorMessage(error) }
                        }
                    },
                    onCancel: {
                        let id = item.vm.id
                        fileSelectionItem = nil
                        Task {
                            do { try await store.remove(id: id, deleteFiles: true) }
                            catch { actionError = engineErrorMessage(error) }
                        }
                    }
                )
            }
    }

    private var engineErrorBinding: Binding<Bool> {
        Binding(
            get: { store.engineStartError != nil },
            set: { if !$0 { store.clearEngineError() } }
        )
    }

    private var actionErrorBinding: Binding<Bool> {
        Binding(
            get: { actionError != nil },
            set: { if !$0 { actionError = nil } }
        )
    }

    // Opens the file-selection modal for a resolved torrent. Skips if a modal is
    // already open (the torrent stays clickable in its awaiting-files row).
    private func presentSelection(forID id: UInt64) {
        guard let vm = store.torrents.first(where: { $0.id == id }) else { return }
        beginFileSelection(for: vm)
    }

    // Fetches the file list and opens the FileSelectionSheet for a given VM.
    // Shared by the magnet auto-present path, the picker, and the drop handler.
    // Skips if a file-selection modal is already open.
    private func beginFileSelection(for vm: TorrentVM) {
        guard fileSelectionItem == nil else { return }
        Task {
            let files = await store.waitForTorrentFiles(id: vm.id)
            // Re-check after the async wait: another call may have opened the sheet.
            guard fileSelectionItem == nil else { return }
            if files.isEmpty {
                actionError = "Could not read the file list for this torrent."
            } else {
                fileSelectionItem = FileSelectionItem(vm: vm, files: files)
            }
        }
    }

    // Presents a system file picker restricted to .torrent files, reads the selected
    // file's bytes, and routes through the existing two-phase add flow.
    private func openTorrentFilePicker() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.title = "Open Torrent File"
        panel.allowedContentTypes = [UTType(filenameExtension: "torrent") ?? .data]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task {
            do {
                let data = try Data(contentsOf: url)
                if let vm = try await store.addTorrentFile(data) {
                    beginFileSelection(for: vm)
                } else {
                    actionError = "This torrent is already in the list."
                }
            } catch {
                actionError = engineErrorMessage(error)
            }
        }
    }

    // Handles a drop onto the torrent list. Accepts .torrent file URLs and magnet text/URLs.
    // Returns true if the drop was accepted (even if the extension check fails later), false if
    // no actionable item was found (triggers OS rejection animation).
    @discardableResult
    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        guard store.isEngineReady, fileSelectionItem == nil else { return false }

        // File drop: iterate all .fileURL providers so a valid .torrent isn't skipped when
        // non-torrent files appear earlier in the drop. Error on non-torrent files is suppressed
        // in multi-file drops to avoid a confusing error+success mix.
        let fileProviders = providers.filter { $0.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) }
        if !fileProviders.isEmpty {
            let suppressNonTorrentError = fileProviders.count > 1
            for provider in fileProviders {
                _ = provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, error in
                    if let error {
                        Task { @MainActor in actionError = error.localizedDescription }
                        return
                    }
                    let url: URL?
                    if let u = item as? NSURL { url = u as URL }
                    else if let d = item as? Data { url = URL(dataRepresentation: d, relativeTo: nil) }
                    else { url = nil }
                    guard let url else { return }
                    guard url.pathExtension.lowercased() == "torrent" else {
                        if !suppressNonTorrentError {
                            Task { @MainActor in actionError = "Only .torrent files can be dropped here." }
                        }
                        return
                    }
                    // Read while the sandbox grant from the drag session is still active
                    // (the grant may not extend to asynchronously dispatched Tasks).
                    let data: Data
                    do { data = try Data(contentsOf: url) } catch {
                        Task { @MainActor in actionError = error.localizedDescription }
                        return
                    }
                    Task { @MainActor in
                        // Guard against multiple .torrent files in the same drop racing to open the sheet.
                        // isHandlingFileDrop is set synchronously before the await so concurrent Tasks
                        // that pass the fileSelectionItem check see the flag and bail out.
                        guard fileSelectionItem == nil, !isHandlingFileDrop else { return }
                        isHandlingFileDrop = true
                        do {
                            if let vm = try await store.addTorrentFile(data) {
                                beginFileSelection(for: vm)
                            } else {
                                actionError = "This torrent is already in the list."
                            }
                        } catch {
                            actionError = engineErrorMessage(error)
                        }
                        isHandlingFileDrop = false
                    }
                }
            }
            return true
        }

        // Text/URL drop: route through the magnet add flow. Providers are tried sequentially
        // so a valid magnet isn't skipped when a non-magnet item appears earlier. For each
        // provider, text is tried first; URL is the fallback if text isn't a magnet link.
        // Only the first valid magnet per drop is added (multi-add is a follow-up).
        let textOrURLProviders = providers.filter {
            $0.hasItemConformingToTypeIdentifier(UTType.text.identifier) ||
            $0.hasItemConformingToTypeIdentifier(UTType.url.identifier)
        }
        guard !textOrURLProviders.isEmpty else { return false }
        Task { @MainActor in
            for provider in textOrURLProviders {
                if let uri = await loadMagnetURI(from: provider) {
                    store.beginAddMagnet(uri)
                    return
                }
            }
            actionError = "Only magnet links are supported."
        }
        return true
    }

    // Returns the first magnet URI from the provider, trying .text before .url.
    private func loadMagnetURI(from provider: NSItemProvider) async -> String? {
        if provider.hasItemConformingToTypeIdentifier(UTType.text.identifier),
           let text = await loadProviderString(from: provider, typeId: UTType.text.identifier),
           text.lowercased().hasPrefix("magnet:") {
            return text
        }
        if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
           let text = await loadProviderString(from: provider, typeId: UTType.url.identifier),
           text.lowercased().hasPrefix("magnet:") {
            return text
        }
        return nil
    }

    private func loadProviderString(from provider: NSItemProvider, typeId: String) async -> String? {
        await withCheckedContinuation { continuation in
            _ = provider.loadItem(forTypeIdentifier: typeId, options: nil) { item, _ in
                let raw: String?
                if let s = item as? String { raw = s }
                else if let d = item as? Data { raw = String(data: d, encoding: .utf8) }
                else if let u = item as? NSURL { raw = u.absoluteString }
                else { raw = nil }
                let trimmed = (raw ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                continuation.resume(returning: trimmed.isEmpty ? nil : trimmed)
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 18) {
            ZStack {
                Circle()
                    .strokeBorder(style: StrokeStyle(lineWidth: 2, dash: [6]))
                    .foregroundStyle(.quaternary)
                    .frame(width: 84, height: 84)
                MagnetIcon()
                    .foregroundStyle(.secondary)
                    .frame(width: 38, height: 40)
                    .scaleEffect(x: 1, y: -1)
            }
            VStack(spacing: 6) {
                Text("Nothing downloading yet")
                    .font(.title2)
                    .fontWeight(.semibold)
                Text("Paste a magnet link or drop a .torrent file")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 12) {
                Button {
                    showAddSheet = true
                } label: {
                    Label("Add magnet link", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(!store.isEngineReady)

                Button {
                    openTorrentFilePicker()
                } label: {
                    Label("Open File", systemImage: "folder")
                }
                .controlSize(.large)
                .disabled(!store.isEngineReady || fileSelectionItem != nil || isHandlingFileDrop)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }
}

// A horseshoe magnet glyph drawn with shapes, since the SF Symbol "magnet"
// is only available on macOS 14+ and the deployment target is macOS 13.0.
struct MagnetIcon: View {
    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let h = geo.size.height
            let lw = w * 0.26
            let r = (w - lw) / 2
            let cx = w / 2
            let top = lw / 2
            let legBottom = h - lw / 2
            let bandY = legBottom - lw * 0.7

            ZStack {
                // U-shaped horseshoe body.
                Path { p in
                    p.move(to: CGPoint(x: lw / 2, y: legBottom))
                    p.addLine(to: CGPoint(x: lw / 2, y: top + r))
                    p.addQuadCurve(to: CGPoint(x: cx, y: top),
                                   control: CGPoint(x: lw / 2, y: top))
                    p.addQuadCurve(to: CGPoint(x: w - lw / 2, y: top + r),
                                   control: CGPoint(x: w - lw / 2, y: top))
                    p.addLine(to: CGPoint(x: w - lw / 2, y: legBottom))
                }
                .stroke(style: StrokeStyle(lineWidth: lw, lineCap: .round, lineJoin: .round))

                // Pole bands near the two tips.
                Path { p in
                    p.move(to: CGPoint(x: 0, y: bandY))
                    p.addLine(to: CGPoint(x: lw, y: bandY))
                    p.move(to: CGPoint(x: w - lw, y: bandY))
                    p.addLine(to: CGPoint(x: w, y: bandY))
                }
                .stroke(style: StrokeStyle(lineWidth: lw * 0.55, lineCap: .butt))
            }
        }
    }
}

struct TorrentRow: View {
    @ObservedObject var vm: TorrentVM
    var onChooseFiles: () -> Void = {}
    @EnvironmentObject private var store: AppStore

    @State private var showRemoveDialog = false
    @State private var rowError: String?

    private static let byteFormatter: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()

    private var canResume: Bool {
        let state = vm.stats?.state
        return state == .paused || state == .error
    }

    private var canPause: Bool {
        let state = vm.stats?.state
        return state == .downloading || state == .seeding
    }

    private func showInFinder() {
        let base = store.downloadDirectory
        let named = base.appendingPathComponent(vm.info.name)
        let target = FileManager.default.fileExists(atPath: named.path) ? named : base
        NSWorkspace.shared.activateFileViewerSelecting([target])
    }



    private var progress: Double {
        guard let stats = vm.stats, stats.totalBytes > 0 else { return 0.0 }
        return min(1.0, Double(stats.downloadedBytes) / Double(stats.totalBytes))
    }

    var body: some View {
        if vm.needsFileSelection {
            awaitingSelectionRow
        } else {
            normalRow
        }
    }

    // Shown after a magnet resolves but before files are chosen. Tapping reopens the
    // file-selection modal (which may have been dismissed without choosing).
    private var awaitingSelectionRow: some View {
        Button(action: onChooseFiles) {
            HStack(spacing: 14) {
                Image(systemName: "checklist")
                    .font(.title2)
                    .foregroundStyle(.orange)
                VStack(alignment: .leading, spacing: 4) {
                    Text(vm.info.name.isEmpty ? "(Unknown)" : vm.info.name)
                        .lineLimit(1)
                    Text("Ready — choose files to download")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
                Spacer()
                Text("Choose files")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(Color.orange.opacity(0.08))
    }

    private var normalRow: some View {
        HStack(spacing: 14) {
            stateButton
            VStack(alignment: .leading, spacing: 6) {
                Text(vm.info.name.isEmpty ? "(Unknown)" : vm.info.name)
                    .lineLimit(1)
                HStack(spacing: 8) {
                    subtitleView
                    Spacer()
                    ProgressView(value: progress, total: 1.0)
                        .frame(width: 120)
                }
            }
        }
        .padding(.vertical, 8)
        .contextMenu {
            if canResume {
                Button {
                    Task {
                        do { try await store.resume(id: vm.id) }
                        catch { print("[BitRufus] resume error: \(error)"); rowError = engineErrorMessage(error) }
                    }
                } label: {
                    Label("Resume", systemImage: "play.fill")
                }
                Divider()
            } else if canPause {
                Button {
                    Task {
                        do { try await store.pause(id: vm.id) }
                        catch { print("[BitRufus] pause error: \(error)"); rowError = engineErrorMessage(error) }
                    }
                } label: {
                    Label("Pause", systemImage: "pause.fill")
                }
                Divider()
            }
            Button {
                showInFinder()
            } label: {
                Label("Show in Finder", systemImage: "folder")
            }
            Divider()
            Button(role: .destructive) {
                showRemoveDialog = true
            } label: {
                Label("Delete…", systemImage: "trash")
            }
        }
        .confirmationDialog(
            vm.info.name.isEmpty ? "Delete torrent?" : vm.info.name,
            isPresented: $showRemoveDialog,
            titleVisibility: .visible
        ) {
            Button("Delete Task", role: .destructive) {
                Task {
                    do { try await store.remove(id: vm.id, deleteFiles: false) }
                    catch { print("[BitRufus] remove error: \(error)"); rowError = engineErrorMessage(error) }
                }
            }
            Button("Delete with Files", role: .destructive) {
                Task {
                    do { try await store.remove(id: vm.id, deleteFiles: true) }
                    catch { print("[BitRufus] remove error: \(error)"); rowError = engineErrorMessage(error) }
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .alert("Error", isPresented: Binding(
            get: { rowError != nil },
            set: { if !$0 { rowError = nil } }
        )) {
            Button("OK", role: .cancel) { rowError = nil }
        } message: {
            Text(rowError ?? "")
        }
    }

    @ViewBuilder
    private var subtitleView: some View {
        if let stats = vm.stats {
            switch stats.state {
            case .downloading:
                Text(downloadingText(stats))
                    .foregroundStyle(.secondary)
                    .font(.caption)
            case .paused:
                let finished = stats.totalBytes > 0 && stats.downloadedBytes >= stats.totalBytes
                Text(finished
                    ? "\(Self.byteFormatter.string(fromByteCount: Int64(clamping: stats.totalBytes))) · Finished"
                    : "\(progressText(stats)) · Paused")
                    .font(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(finished ? .green : .orange)
            case .seeding:
                Text("\(Self.byteFormatter.string(fromByteCount: Int64(clamping: stats.totalBytes))) · Seeding")
                    .font(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(.green)
            case .error:
                Text("Error")
                    .font(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(.red)
            case .initializing:
                sizeText
            }
        } else {
            sizeText
        }
    }

    private var sizeText: some View {
        Text(vm.info.totalBytes > 0
            ? Self.byteFormatter.string(fromByteCount: Int64(clamping: vm.info.totalBytes))
            : "Fetching…")
            .foregroundStyle(.secondary)
            .font(.caption)
    }

    private func progressText(_ stats: TorrentStats) -> String {
        let down = Self.byteFormatter.string(fromByteCount: Int64(clamping: stats.downloadedBytes))
        let total = Self.byteFormatter.string(fromByteCount: Int64(clamping: stats.totalBytes))
        return "\(down) / \(total)"
    }

    private func downloadingText(_ stats: TorrentStats) -> String {
        let speed = Self.byteFormatter.string(fromByteCount: Int64(clamping: stats.downloadSpeedBps))
        var parts = [progressText(stats), "\(speed)/s"]
        if let eta = etaText(stats) { parts.append(eta) }
        if stats.peerCount > 0 { parts.append("\(stats.peerCount) peers") }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private var stateButton: some View {
        switch vm.stats?.state {
        case .downloading:
            Button {
                Task {
                    do { try await store.pause(id: vm.id) }
                    catch { rowError = engineErrorMessage(error) }
                }
            } label: {
                Image(systemName: "pause.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.blue)
            }
            .buttonStyle(.plain)
        case .paused, .error:
            Button {
                Task {
                    do { try await store.resume(id: vm.id) }
                    catch { rowError = engineErrorMessage(error) }
                }
            } label: {
                Image(systemName: "play.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.blue)
            }
            .buttonStyle(.plain)
        case .seeding:
            Image(systemName: "checkmark.circle.fill")
                .font(.title2)
                .foregroundStyle(.green)
        case .initializing, nil:
            Image(systemName: "circle.dotted")
                .font(.title2)
                .foregroundStyle(.secondary)
        }
    }

    private func etaText(_ stats: TorrentStats) -> String? {
        guard stats.downloadSpeedBps > 0, stats.totalBytes > stats.downloadedBytes else { return nil }
        let seconds = Int((stats.totalBytes - stats.downloadedBytes) / stats.downloadSpeedBps)
        if seconds < 60 {
            return "\(seconds)s"
        } else if seconds < 3600 {
            return "\(seconds / 60)m \(seconds % 60)s"
        } else {
            let h = seconds / 3600
            let m = (seconds % 3600) / 60
            return "\(h)h \(m)m"
        }
    }
}

// A magnet whose metadata is still resolving (or has failed to resolve). Shown as a
// placeholder row above the real torrents until it becomes a TorrentVM or is dismissed.
struct PendingMagnetRow: View {
    @ObservedObject var pending: PendingMagnet
    let onCancel: () -> Void
    let onRetry: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            statusIcon
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 5) {
                Text(pending.name)
                    .lineLimit(1)
                switch pending.state {
                case .resolving:
                    Text("Looking for peers…")
                        .font(.caption)
                        .foregroundStyle(.blue)
                case .failed(let reason):
                    Text(reason)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(2)
                }
            }
            Spacer()
            if case .failed = pending.state {
                Button("Retry", action: onRetry)
            }
            Button(action: onCancel) {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .help("Remove")
        }
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch pending.state {
        case .resolving:
            ProgressView()
                .controlSize(.small)
        case .failed:
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.title3)
                .foregroundStyle(.red)
        }
    }
}
