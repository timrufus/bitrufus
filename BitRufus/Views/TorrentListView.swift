import SwiftUI

struct TorrentListView: View {
    @EnvironmentObject private var store: AppStore
    @State private var showAddSheet = false
    @State private var pendingVM: TorrentVM?
    @State private var pendingFiles: [FileInfo] = []
    @State private var showFileSelection = false
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
                    pendingFiles = []
                    pendingFiles = await store.waitForTorrentFiles(id: vm.id)
                    if pendingFiles.isEmpty {
                        pendingVM = nil
                        do {
                            try await store.cancelTorrent(vm.id)
                            actionError = "Could not fetch torrent metadata. The magnet link may be invalid or unreachable."
                        } catch {
                            store.confirmTorrent(vm)
                            actionError = engineErrorMessage(error)
                        }
                    } else {
                        showFileSelection = true
                    }
                }
            }
        }) {
            AddMagnetSheet { vm in
                pendingVM = vm
            }
        }
        .sheet(isPresented: $showFileSelection, onDismiss: {
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
        }) {
            if let vm = pendingVM {
                FileSelectionSheet(
                    vm: vm,
                    files: pendingFiles,
                    onConfirm: { selectedIndexes in
                        let id = vm.id
                        pendingVM = nil
                        showFileSelection = false
                        Task {
                            do {
                                try await store.setFileSelection(id: id, selectedIndexes: selectedIndexes)
                                store.confirmTorrent(vm)
                            } catch let selectionError {
                                do {
                                    try await store.cancelTorrent(id)
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
                        pendingVM = nil
                        showFileSelection = false
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

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(vm.info.name.isEmpty ? "(Unknown)" : vm.info.name)
                .lineLimit(1)
            HStack {
                Text(vm.info.totalBytes > 0
                    ? Self.byteFormatter.string(fromByteCount: Int64(clamping: vm.info.totalBytes))
                    : "Fetching…")
                    .foregroundStyle(.secondary)
                    .font(.caption)
                Spacer()
                // Progress wired in a later plan; 0% placeholder until stats are available.
                ProgressView(value: 0.0, total: 1.0)
                    .frame(width: 120)
            }
        }
        .padding(.vertical, 2)
    }
}
