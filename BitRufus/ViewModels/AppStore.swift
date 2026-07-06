import Foundation
import Combine

// TorrentVM holds the latest info and optional stats for one torrent.
// Stats are populated by the 500ms polling loop in AppStore; nil until the first poll completes.
@MainActor
final class TorrentVM: ObservableObject, Identifiable {
    let id: UInt64
    @Published private(set) var info: TorrentInfo
    @Published private(set) var stats: TorrentStats?
    // True after a magnet resolves but before the user has chosen which files to
    // download. The torrent stays paused and the row is highlighted until selection.
    @Published var needsFileSelection: Bool = false

    init(info: TorrentInfo) {
        self.id = info.id
        self.info = info
    }

    func updateStats(_ newStats: TorrentStats) {
        guard stats != newStats else { return }
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

// A magnet that has been pasted but whose metadata is still being resolved from
// peers/DHT/trackers. Shown as a placeholder row until it resolves into a TorrentVM
// (success) or fails (timeout / no peers).
enum PendingMagnetState: Equatable {
    case resolving
    case failed(String)
}

@MainActor
final class PendingMagnet: ObservableObject, Identifiable {
    let id = UUID()
    let uri: String
    @Published var name: String
    @Published var state: PendingMagnetState = .resolving
    // Set when the user cancels while the (non-abortable) resolve is still in flight,
    // so the resolved torrent is removed from the engine once the add finally returns.
    var cancelled = false
    // When the current resolve attempt started, for showing elapsed time in the row.
    var startedAt = Date()
    // Which auto-retry attempt is currently running (1-based); shown in the row after the first.
    @Published var attempt = 1

    init(uri: String, name: String) {
        self.uri = uri
        self.name = name
    }
}

// AppStore owns the singleton Engine and the observable list of torrents.
// Engine.init is async, so we start it in a background Task from init().
@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var torrents: [TorrentVM] = []
    @Published private(set) var pendingMagnets: [PendingMagnet] = []
    @Published private(set) var engineStartError: String?
    @Published private(set) var isEngineReady: Bool = false
    // Engine id of a just-resolved torrent whose file-selection modal should auto-open.
    // The view consumes and clears it. nil when nothing is waiting to auto-present.
    @Published var autoPresentSelectionFor: UInt64?

    private var engine: Engine?
    private var statsPollingTask: Task<Void, Never>?
    private let torrentStore = TorrentStore()
    private var isSceneActive: Bool = true
    var downloadDirectory: URL { AppSettings.shared.downloadDirectory }

    init() {
        Task {
            await startEngine()
        }
    }

    deinit {
        statsPollingTask?.cancel()
    }

    // Called by the view when scenePhase changes. Keeps isSceneActive in sync so that
    // startEngine() can re-enable polling correctly even when no window is open.
    func updateScenePhase(active: Bool) {
        isSceneActive = active
        setPolling(active: active)
    }

    // Starts or stops the 0.5 s stats-polling loop. Call with active: true when the
    // scene becomes .active and active: false when it goes .inactive/.background so the
    // thread can truly idle rather than just no-op inside the loop. Always cancels any
    // existing task before starting a new one, so rapid toggles are safe.
    func setPolling(active: Bool) {
        statsPollingTask?.cancel()
        statsPollingTask = nil
        guard active else { return }
        statsPollingTask = Task { [weak self] in
            for await _ in Timer.publish(every: 0.5, on: .main, in: .common).autoconnect().values {
                guard let self else { return }
                self.refreshStats()
            }
        }
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
            setPolling(active: isSceneActive)
        } catch {
            engineStartError = engineErrorMessage(error)
        }
    }

    // Pastes a magnet and shows it as a resolving placeholder row immediately, then
    // resolves its metadata in the background (engine.addMagnet blocks until librqbit
    // fetches the file list from peers/DHT/trackers). On success it becomes a TorrentVM
    // in `torrents`, paused and awaiting file selection. On failure the placeholder is
    // marked .failed so the user can retry or dismiss it.
    func beginAddMagnet(_ uri: String) {
        let pending = PendingMagnet(uri: uri, name: Self.magnetDisplayName(uri) ?? "Magnet link")
        pendingMagnets.append(pending)
        Task { await resolvePending(pending) }
    }

    // Number of automatic resolve attempts before giving up and offering a manual Retry.
    // Tracker DNS blocks are frequently intermittent, so simply re-attempting usually
    // catches a working window — this is effectively what clients like Folx do. Kept
    // generous (~40 min of attempts at 120s each) since the loop is cheap and cancellable.
    private static let magnetResolveAttempts = 20

    private func resolvePending(_ pending: PendingMagnet) async {
        guard let engine else {
            pending.state = .failed("engine not initialized")
            return
        }
        while true {
            pending.startedAt = Date()
            do {
                let info = try await engine.addMagnet(magnet: pending.uri)
                pendingMagnets.removeAll { $0.id == pending.id }
                // The user cancelled while the resolve was still running: undo the engine add,
                // but only if the torrent wasn't subsequently re-added (e.g. via .torrent file)
                // before this resolution completed — in that case leave the live torrent alone.
                if pending.cancelled {
                    if !torrents.contains(where: { $0.id == info.id }) {
                        try? await engine.remove(id: info.id, deleteFiles: true)
                    }
                    return
                }
                // Duplicate of an already-listed torrent: drop the placeholder silently.
                if torrents.contains(where: { $0.id == info.id }) { return }
                let vm = TorrentVM(info: info)
                vm.needsFileSelection = true
                torrents.append(vm)
                torrentStore.record(id: vm.id, meta: TorrentMeta(displayName: vm.info.name, addedAt: Date()))
                autoPresentSelectionFor = vm.id
                return
            } catch {
                // Cancelled while the (non-abortable) attempt was in flight: stop quietly.
                if pending.cancelled { return }
                // Out of automatic attempts: surface the error with a manual Retry button.
                guard pending.attempt < Self.magnetResolveAttempts else {
                    pending.state = .failed(engineErrorMessage(error))
                    return
                }
                // Brief pause, then retry — the block is often intermittent.
                pending.attempt += 1
                do { try await Task.sleep(nanoseconds: 10_000_000_000) } catch { return }
                if pending.cancelled { return }
            }
        }
    }

    // Adds a .torrent file by its raw bytes. Returns nil if the torrent is already in
    // the list (duplicate), or a new TorrentVM paused and awaiting file selection.
    // Unlike addMagnet, metadata is embedded in the file so no pollMetadata is needed.
    func addTorrentFile(_ data: Data) async throws -> TorrentVM? {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        let info = try await engine.addTorrentFile(bytes: data)
        if torrents.contains(where: { $0.id == info.id }) { return nil }
        let vm = TorrentVM(info: info)
        vm.needsFileSelection = true
        torrents.append(vm)
        torrentStore.record(id: vm.id, meta: TorrentMeta(displayName: vm.info.name, addedAt: Date()))
        return vm
    }

    // Re-runs a failed resolve.
    func retryPending(_ pending: PendingMagnet) {
        pending.state = .resolving
        pending.cancelled = false
        pending.attempt = 1
        pending.startedAt = Date()
        Task { await resolvePending(pending) }
    }

    // Removes a placeholder row. If a resolve is still in flight it cannot be aborted,
    // so we tombstone it; resolvePending removes the resulting torrent when it returns.
    func cancelPending(_ pending: PendingMagnet) {
        pending.cancelled = true
        pendingMagnets.removeAll { $0.id == pending.id }
    }

    // Applies the chosen files and starts a torrent that was awaiting file selection.
    func applyFileSelection(id: UInt64, selectedIndexes: [UInt32]) async throws {
        guard let engine else { throw EngineError.Backend(reason: "engine not initialized") }
        try await engine.setFileSelection(id: id, selectedIndexes: selectedIndexes)
        try await engine.resume(id: id)
        torrents.first(where: { $0.id == id })?.needsFileSelection = false
    }

    // Pulls the user-facing name out of a magnet's dn= parameter, percent-decoded.
    private static func magnetDisplayName(_ uri: String) -> String? {
        guard let comps = URLComponents(string: uri),
              let dn = comps.queryItems?.first(where: { $0.name == "dn" })?.value,
              !dn.isEmpty else { return nil }
        return dn
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
                guard let info = try? engine.torrentInfo(id: id) else { return }
                if info.totalBytes > 0 {
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
        let statsMap = Dictionary(uniqueKeysWithValues: engine.allStats().map { ($0.id, $0) })
        for vm in torrents {
            if let s = statsMap[vm.id] {
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
        setPolling(active: false)
        engine = nil
        torrents = []
        isEngineReady = false
        engineStartError = nil
        Task { await startEngine() }
    }
}

func engineErrorMessage(_ error: Error) -> String {
    if case EngineError.Backend(let reason) = error { return reason }
    if case EngineError.InvalidMagnet(let reason) = error { return reason }
    if case EngineError.InvalidTorrent(let reason) = error { return "invalid torrent file: \(reason)" }
    if case EngineError.Io(let reason) = error { return reason }
    if case EngineError.NotFound(let id) = error { return "torrent not found: \(id)" }
    return error.localizedDescription
}
