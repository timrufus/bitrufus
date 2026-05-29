import Foundation

struct TorrentMeta: Codable {
    var displayName: String
    var addedAt: Date
}

// Versioned top-level container. Add new optional fields to extend without schema breakage.
private struct StoreData: Codable {
    var version: Int = 1
    var torrents: [String: TorrentMeta] = [:]
}

final class TorrentStore {
    private let url: URL
    private var data = StoreData()

    init() {
        let appSupport = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first ?? FileManager.default.temporaryDirectory
        let dir = appSupport
            .appendingPathComponent("BitRufus")
            .appendingPathComponent("BitRufus")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        url = dir.appendingPathComponent("torrents.json")
        load()
    }

    func lookup(id: UInt64) -> TorrentMeta? {
        data.torrents["\(id)"]
    }

    func record(id: UInt64, meta: TorrentMeta) {
        data.torrents["\(id)"] = meta
        save()
    }

    func remove(id: UInt64) {
        data.torrents.removeValue(forKey: "\(id)")
        save()
    }

    // Removes entries for ids no longer known to the engine and re-saves if anything changed.
    func dropOrphans(keeping knownIds: Set<UInt64>) {
        let before = data.torrents.count
        data.torrents = data.torrents.filter { key, _ in
            UInt64(key).map { knownIds.contains($0) } ?? false
        }
        if data.torrents.count != before { save() }
    }

    private func load() {
        guard let raw = try? Data(contentsOf: url) else { return }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        if let decoded = try? decoder.decode(StoreData.self, from: raw) {
            data = decoded
        }
    }

    private func save() {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let raw = try? encoder.encode(data) else { return }
        try? raw.write(to: url, options: .atomic)
    }
}
