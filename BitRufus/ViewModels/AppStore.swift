import Foundation

// TorrentVM holds the latest info and optional stats for one torrent.
// Stats are populated by a later plan; nil here is expected.
@MainActor
final class TorrentVM: ObservableObject, Identifiable {
    let id: UInt64
    @Published private(set) var info: TorrentInfo
    @Published private(set) var stats: TorrentStats?

    init(info: TorrentInfo) {
        self.id = info.id
        self.info = info
    }

    // Called by AppStore once metadata resolves (total_bytes and name become available).
    // Preserves the existing name (e.g. dn= display name) if the updated info has an empty name.
    func refreshInfo(_ updated: TorrentInfo) {
        guard updated.totalBytes > 0 else { return }
        let name = updated.name.isEmpty ? info.name : updated.name
        info = TorrentInfo(id: updated.id, name: name, totalBytes: updated.totalBytes)
    }
}

// AppStore owns the singleton Engine and the observable list of torrents.
// Engine.init is async, so we start it in a background Task from init().
@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var torrents: [TorrentVM] = []
    @Published private(set) var engineStartError: String?
    @Published private(set) var isEngineReady: Bool = false

    private var engine: Engine?

    init() {
        Task {
            await startEngine()
        }
    }

    private func startEngine() async {
        let downloads = (FileManager.default
            .urls(for: .downloadsDirectory, in: .userDomainMask)
            .first ?? FileManager.default.temporaryDirectory)
            .appendingPathComponent("TorrentApp")
            .path
        do {
            let e = try await Engine(downloadDir: downloads)
            engine = e
            torrents = e.listTorrents().map { TorrentVM(info: $0) }
            // Restored torrents whose metadata hasn't resolved yet (no size) need the same
            // background polling that addMagnet kicks off for freshly-added magnets.
            for vm in torrents where vm.info.totalBytes == 0 {
                Task { await self.pollMetadata(for: vm.id, engine: e) }
            }
            isEngineReady = true
        } catch {
            engineStartError = engineErrorMessage(error)
        }
    }

    // Adds a magnet to the engine but does NOT append to `torrents`.
    // Returns nil if the torrent is already confirmed (duplicate magnet).
    // Caller must call confirmTorrent(_:) or cancelTorrent(_:) on the returned VM.
    func addMagnet(_ uri: String) async throws -> TorrentVM? {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        let info = try await engine.addMagnet(magnet: uri)
        if torrents.contains(where: { $0.id == info.id }) { return nil }
        return TorrentVM(info: info)
    }

    func confirmTorrent(_ vm: TorrentVM) {
        guard !torrents.contains(where: { $0.id == vm.id }) else { return }
        torrents.append(vm)
        guard let engine, vm.info.totalBytes == 0 else { return }
        Task { await pollMetadata(for: vm.id, engine: engine) }
    }

    func cancelTorrent(_ id: UInt64) async throws {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        try await engine.remove(id: id, deleteFiles: true)
    }

    func setFileSelection(id: UInt64, selectedIndexes: [UInt32]) async throws {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        try await engine.setFileSelection(id: id, selectedIndexes: selectedIndexes)
    }

    func torrentFiles(id: UInt64) -> [FileInfo] {
        return (try? engine?.torrentFiles(id: id)) ?? []
    }

    // Polls until torrent_files returns a non-empty list (metadata resolved) or 30 s timeout.
    // Magnet-only adds have no file metadata until DHT/trackers respond; calling torrentFiles
    // immediately after addMagnet always returns empty.
    func waitForTorrentFiles(id: UInt64) async -> [FileInfo] {
        for _ in 0..<60 {
            let files = (try? engine?.torrentFiles(id: id)) ?? []
            if !files.isEmpty { return files }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        return []
    }

    // Polls until total_bytes > 0 (metadata resolved) or the window expires.
    // Phase 1: 0.5 s × 60 = 30 s (typical case). Phase 2: 5 s × 60 = 5 min (slow DHT).
    private func pollMetadata(for id: UInt64, engine: Engine) async {
        let phases: [(count: Int, nanos: UInt64)] = [(60, 500_000_000), (60, 5_000_000_000)]
        for (count, nanos) in phases {
            for _ in 0..<count {
                do {
                    try await Task.sleep(nanoseconds: nanos)
                } catch {
                    return
                }
                guard let vm = torrents.first(where: { $0.id == id }) else { return }
                if let info = engine.listTorrents().first(where: { $0.id == id }),
                   info.totalBytes > 0 {
                    vm.refreshInfo(info)
                    return
                }
            }
        }
    }

    func clearEngineError() {
        engineStartError = nil
    }
}

func engineErrorMessage(_ error: Error) -> String {
    if case EngineError.Backend(let reason) = error { return reason }
    if case EngineError.InvalidMagnet(let reason) = error { return reason }
    if case EngineError.Io(let reason) = error { return reason }
    if case EngineError.NotFound(let id) = error { return "torrent not found: \(id)" }
    return error.localizedDescription
}
