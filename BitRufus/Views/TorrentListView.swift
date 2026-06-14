import SwiftUI
import AppKit

struct FileSelectionItem: Identifiable {
    let vm: TorrentVM
    let files: [FileInfo]
    var id: UInt64 { vm.id }
}

struct TorrentListView: View {
    @EnvironmentObject private var store: AppStore
    @State private var showAddSheet = false
    @State private var pendingVM: TorrentVM?
    @State private var fileSelectionItem: FileSelectionItem?
    @State private var actionError: String?
    @State private var showDiskSpace = false

    var body: some View {
        Group {
            if store.torrents.isEmpty {
                emptyState
            } else {
                List(store.torrents) { vm in
                    TorrentRow(vm: vm)
                }
                .scrollContentBackground(.hidden)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
        .toolbar {
            ToolbarItem(placement: .automatic) {
                Button {
                    showDiskSpace.toggle()
                } label: {
                    Label("Disk Space", systemImage: "internaldrive")
                }
                .popover(isPresented: $showDiskSpace) {
                    DiskSpacePopover()
                }
            }
            ToolbarItem {
                Button {
                    showAddSheet = true
                } label: {
                    Label("Add Torrent", systemImage: "plus")
                }
                .disabled(!store.isEngineReady || pendingVM != nil)
            }
        }
        .sheet(isPresented: $showAddSheet, onDismiss: {
            if let vm = pendingVM {
                Task {
                    let files = await store.waitForTorrentFiles(id: vm.id)
                    if files.isEmpty {
                        pendingVM = nil
                        do {
                            try await store.cancelTorrent(vm.id)
                            actionError = "Could not fetch torrent metadata. The magnet link may be invalid or unreachable."
                        } catch {
                            store.confirmTorrent(vm)
                            actionError = engineErrorMessage(error)
                        }
                    } else {
                        fileSelectionItem = FileSelectionItem(vm: vm, files: files)
                    }
                }
            }
        }) {
            AddMagnetSheet { vm in
                pendingVM = vm
            }
        }
        // Attach the file-selection sheet to a background EmptyView so both sheets
        // coexist on macOS 13.0–13.2 (only one .sheet per view was supported before
        // macOS 13.3; attaching to a separate view node avoids the conflict).
        .background(
            EmptyView()
                .sheet(item: $fileSelectionItem, onDismiss: {
                    if let vm = pendingVM {
                        pendingVM = nil
                        Task {
                            do {
                                try await store.cancelTorrent(vm.id)
                            } catch {
                                store.confirmTorrent(vm)
                                actionError = engineErrorMessage(error)
                            }
                        }
                    }
                }) { item in
                    FileSelectionSheet(
                        vm: item.vm,
                        files: item.files,
                        onConfirm: { selectedIndexes in
                            let vm = item.vm
                            pendingVM = nil
                            fileSelectionItem = nil
                            Task {
                                do {
                                    try await store.setFileSelection(id: vm.id, selectedIndexes: selectedIndexes)
                                    store.confirmTorrent(vm)
                                } catch let selectionError {
                                    do {
                                        try await store.cancelTorrent(vm.id)
                                    } catch {
                                        store.confirmTorrent(vm)
                                        actionError = "Failed to apply file selection. Torrent added to your list."
                                        return
                                    }
                                    actionError = engineErrorMessage(selectionError)
                                }
                            }
                        },
                        onCancel: {
                            let vm = item.vm
                            pendingVM = nil
                            fileSelectionItem = nil
                            Task {
                                do {
                                    try await store.cancelTorrent(vm.id)
                                } catch {
                                    store.confirmTorrent(vm)
                                    actionError = engineErrorMessage(error)
                                }
                            }
                        }
                    )
                }
        )
        .alert("Engine Error", isPresented: Binding(
            get: { store.engineStartError != nil },
            set: { if !$0 { store.clearEngineError() } }
        )) {
            Button("OK", role: .cancel) { store.clearEngineError() }
        } message: {
            Text(store.engineStartError ?? "")
        }
        .alert("Error", isPresented: Binding(
            get: { actionError != nil },
            set: { if !$0 { actionError = nil } }
        )) {
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
                Text("Paste a magnet link here")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            Button {
                showAddSheet = true
            } label: {
                Label("Add magnet link", systemImage: "plus")
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(!store.isEngineReady || pendingVM != nil)
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
