import AppKit
import UserNotifications

// Posts a local "download finished" notification with a "Show in Finder" action.
// Authorization is requested lazily on the first completed download; the delegate is
// installed at app launch (BitRufusApp.init) so notification clicks are handled even
// when the app was relaunched by the click.
final class DownloadNotifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = DownloadNotifier()

    private static let categoryId = "downloadFinished"
    private static let showInFinderActionId = "showInFinder"
    private static let pathKey = "path"

    // Installs self as the notification-center delegate and registers the category
    // with its action button. Must run before the app finishes launching.
    func activate() {
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let show = UNNotificationAction(
            identifier: Self.showInFinderActionId,
            title: "Show in Finder"
        )
        let category = UNNotificationCategory(
            identifier: Self.categoryId,
            actions: [show],
            intentIdentifiers: []
        )
        center.setNotificationCategories([category])
    }

    func notifyDownloadFinished(name: String, path: String) {
        let center = UNUserNotificationCenter.current()
        // No-op prompt after the user has decided once; on denial we silently skip.
        center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
            guard granted else { return }
            let content = UNMutableNotificationContent()
            content.title = "Download finished"
            content.body = name
            content.sound = .default
            content.categoryIdentifier = Self.categoryId
            content.userInfo = [Self.pathKey: path]
            center.add(UNNotificationRequest(
                identifier: "downloadFinished-\(path)",
                content: content,
                trigger: nil
            ))
        }
    }

    // Show the banner even when the app is frontmost.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }

    // Both the "Show in Finder" button and a plain click on the banner reveal the
    // downloaded file (or the download directory if the file has moved).
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        guard response.actionIdentifier == Self.showInFinderActionId
            || response.actionIdentifier == UNNotificationDefaultActionIdentifier,
            let path = response.notification.request.content.userInfo[Self.pathKey] as? String
        else { return }
        let url = URL(fileURLWithPath: path)
        let target = FileManager.default.fileExists(atPath: url.path)
            ? url
            : url.deletingLastPathComponent()
        await MainActor.run {
            NSWorkspace.shared.activateFileViewerSelecting([target])
        }
    }
}
