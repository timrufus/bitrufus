import SwiftUI

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

    var body: some View {
        List(store.torrents) { vm in
            TorrentRow(vm: vm)
        }
        .toolbar {
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
                    fileSelectionItem = nil
                    pendingVM = nil
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
                    fileSelectionItem = nil
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
            )
        }
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
        .frame(minWidth: 500, minHeight: 300)
    }
}

struct TorrentRow: View {
    @ObservedObject var vm: TorrentVM

    private static let byteFormatter: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()

    private var progress: Double {
        guard let stats = vm.stats, stats.totalBytes > 0 else { return 0.0 }
        return Double(stats.downloadedBytes) / Double(stats.totalBytes)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(vm.info.name.isEmpty ? "(Unknown)" : vm.info.name)
                .lineLimit(1)
            HStack(spacing: 8) {
                subtitleView
                Spacer()
                ProgressView(value: progress, total: 1.0)
                    .frame(width: 120)
            }
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder
    private var subtitleView: some View {
        if let stats = vm.stats {
            switch stats.state {
            case .downloading:
                Text(speedAndPeers(stats))
                    .foregroundStyle(.secondary)
                    .font(.caption)
            case .paused:
                Text("Paused")
                    .font(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(.orange)
            case .seeding:
                Text("Seeding")
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

    private func speedAndPeers(_ stats: TorrentStats) -> String {
        let speed = Self.byteFormatter.string(fromByteCount: Int64(clamping: stats.downloadSpeedBps))
        return "\(speed)/s · \(stats.peerCount) peers"
    }
}
