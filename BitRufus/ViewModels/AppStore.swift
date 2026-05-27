import Foundation

// TorrentVM holds the latest info and optional stats for one torrent.
// Stats are populated by a later plan; nil here is expected.
final class TorrentVM: ObservableObject, Identifiable {
    let id: UInt64
    @Published private(set) var info: TorrentInfo
    @Published private(set) var stats: TorrentStats?

    init(info: TorrentInfo) {
        self.id = info.id
        self.info = info
    }
}

// AppStore owns the singleton Engine and the observable list of torrents.
// Engine.init is async, so we start it in a background Task from init().
@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var torrents: [TorrentVM] = []

    private var engine: Engine?

    init() {
        Task {
            await startEngine()
        }
    }

    private func startEngine() async {
        let downloads = FileManager.default
            .urls(for: .downloadsDirectory, in: .userDomainMask)
            .first!
            .appendingPathComponent("TorrentApp")
            .path
        do {
            let e = try await Engine(downloadDir: downloads)
            engine = e
            torrents = e.listTorrents().map { TorrentVM(info: $0) }
        } catch {
            // Engine failed to start; torrents stays empty.
        }
    }

    func addMagnet(_ uri: String) async throws {
        guard let engine else { return }
        let info = try await engine.addMagnet(magnet: uri)
        torrents.append(TorrentVM(info: info))
    }
}
