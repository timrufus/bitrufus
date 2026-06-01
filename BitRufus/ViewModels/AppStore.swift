import Foundation
import Combine

// TorrentVM holds the latest info and optional stats for one torrent.
// Stats are populated by the 500ms polling loop in AppStore; nil until the first poll completes.
@MainActor
final class TorrentVM: ObservableObject, Identifiable {
    let id: UInt64
    @Published private(set) var info: TorrentInfo
    @Published private(set) var stats: TorrentStats?

    init(info: TorrentInfo) {
        self.id = info.id
        self.info = info
    }

    func updateStats(_ newStats: TorrentStats) {
        stats = newStats
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
    private var statsPollingTask: Task<Void, Never>?
    private let torrentStore = TorrentStore()
    var downloadDirectory: URL { AppSettings.shared.downloadDirectory }

    init() {
        Task {
            await startEngine()
        }
        statsPollingTask = Task { [weak self] in
            for await _ in Timer.publish(every: 0.5, on: .main, in: .common).autoconnect().values {
                guard let self else { return }
                self.refreshStats()
            }
        }
    }

    deinit {
        statsPollingTask?.cancel()
    }

    private func startEngine() async {
        let downloadURL = AppSettings.shared.downloadDirectory
        try? FileManager.default.createDirectory(at: downloadURL, withIntermediateDirectories: true)
        let downloads = downloadURL.path
        do {
            let e = try await Engine(downloadDir: downloads)
            engine = e
            torrents = e.listTorrents().map { info in
                let name = info.name.isEmpty
                    ? (torrentStore.lookup(id: info.id)?.displayName ?? "")
                    : info.name
                return TorrentVM(info: TorrentInfo(id: info.id, name: name, totalBytes: info.totalBytes))
            }
            torrentStore.dropOrphans(keeping: Set(torrents.map { $0.id }))
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
        torrentStore.record(id: vm.id, meta: TorrentMeta(displayName: vm.info.name, addedAt: Date()))
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
            do { try await Task.sleep(nanoseconds: 500_000_000) } catch { return [] }
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
                    // Persist the resolved display name so it survives future restarts.
                    let addedAt = torrentStore.lookup(id: id)?.addedAt ?? Date()
                    torrentStore.record(id: id, meta: TorrentMeta(displayName: vm.info.name, addedAt: addedAt))
                    return
                }
            }
        }
    }

    private func refreshStats() {
        guard let engine else { return }
        for vm in torrents {
            if let s = try? engine.torrentStats(id: vm.id) {
                vm.updateStats(s)
            }
        }
    }

    func pause(id: UInt64) async throws {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        try await engine.pause(id: id)
    }

    func resume(id: UInt64) async throws {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        try await engine.resume(id: id)
    }

    func remove(id: UInt64, deleteFiles: Bool) async throws {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        try await engine.remove(id: id, deleteFiles: deleteFiles)
        torrents.removeAll { $0.id == id }
        torrentStore.remove(id: id)
    }

    func clearEngineError() {
        engineStartError = nil
    }

    func restartEngine() {
        statsPollingTask?.cancel()
        statsPollingTask = nil
        engine = nil
        torrents = []
        isEngineReady = false
        engineStartError = nil
        statsPollingTask = Task { [weak self] in
            for await _ in Timer.publish(every: 0.5, on: .main, in: .common).autoconnect().values {
                guard let self else { return }
                self.refreshStats()
            }
        }
        Task { await startEngine() }
    }
}

func engineErrorMessage(_ error: Error) -> String {
    if case EngineError.Backend(let reason) = error { return reason }
    if case EngineError.InvalidMagnet(let reason) = error { return reason }
    if case EngineError.Io(let reason) = error { return reason }
    if case EngineError.NotFound(let id) = error { return "torrent not found: \(id)" }
    return error.localizedDescription
}
