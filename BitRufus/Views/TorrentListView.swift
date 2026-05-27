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
            }
        }
        .sheet(isPresented: $showAddSheet) {
            AddMagnetSheet()
                .environmentObject(store)
        }
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
            Text(vm.info.name)
                .lineLimit(1)
            HStack {
                Text(Self.byteFormatter.string(fromByteCount: Int64(vm.info.totalBytes)))
                    .foregroundStyle(.secondary)
                    .font(.caption)
                Spacer()
                ProgressView(value: 0)
                    .frame(width: 120)
            }
        }
        .padding(.vertical, 2)
    }
}
