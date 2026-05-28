import SwiftUI

struct TorrentListView: View {
    @EnvironmentObject private var store: AppStore
    @State private var showAddSheet = false

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
                .disabled(!store.isEngineReady)
            }
        }
        .sheet(isPresented: $showAddSheet) {
            AddMagnetSheet()
        }
        .alert("Engine Error", isPresented: Binding(
            get: { store.engineStartError != nil },
            set: { if !$0 { store.clearEngineError() } }
        )) {
            Button("OK", role: .cancel) { store.clearEngineError() }
        } message: {
            Text(store.engineStartError ?? "")
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
