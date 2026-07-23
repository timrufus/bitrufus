import Foundation

final class AppSettings {
    static let shared = AppSettings()
    private let bookmarkKey = "downloadDirBookmark"
    private let pathKey = "downloadDirPath"
    // The URL whose security-scoped access is currently active. Access must be started
    // after every bookmark resolve (not just when saving) and kept for the app's
    // lifetime — otherwise the sandbox silently denies writes to folders outside
    // ~/Downloads on the next launch and torrents there fail with storage errors.
    private var activeScopedURL: URL?

    var downloadDirectory: URL {
        if let data = UserDefaults.standard.data(forKey: bookmarkKey),
           let url = resolveBookmark(data) {
            return url
        }
        return defaultDownloadDirectory
    }

    var displayPath: String {
        UserDefaults.standard.string(forKey: pathKey) ?? downloadDirectory.path
    }

    func saveDirectory(_ url: URL) {
        guard url.startAccessingSecurityScopedResource() else { return }
        defer { url.stopAccessingSecurityScopedResource() }
        if let data = try? url.bookmarkData(
            options: .withSecurityScope,
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        ) {
            UserDefaults.standard.set(data, forKey: bookmarkKey)
            UserDefaults.standard.set(url.path, forKey: pathKey)
        }
    }

    private var defaultDownloadDirectory: URL {
        (FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory)
            .appendingPathComponent("BitRufusTorrent")
    }

    private func resolveBookmark(_ data: Data) -> URL? {
        var stale = false
        guard let url = try? URL(
            resolvingBookmarkData: data,
            options: .withSecurityScope,
            relativeTo: nil,
            bookmarkDataIsStale: &stale
        ) else { return nil }
        if activeScopedURL != url {
            activeScopedURL?.stopAccessingSecurityScopedResource()
            guard url.startAccessingSecurityScopedResource() else {
                activeScopedURL = nil
                return nil
            }
            activeScopedURL = url
        }
        if stale, let fresh = try? url.bookmarkData(
            options: .withSecurityScope,
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        ) {
            UserDefaults.standard.set(fresh, forKey: bookmarkKey)
        }
        return url
    }
}
