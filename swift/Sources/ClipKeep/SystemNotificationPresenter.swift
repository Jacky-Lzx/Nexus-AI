import Foundation
import UserNotifications

protocol SystemNotificationCenter: AnyObject {
    var delegate: UNUserNotificationCenterDelegate? { get set }
    func authorizationStatus(completion: @escaping (UNAuthorizationStatus) -> Void)
    func requestAuthorization(options: UNAuthorizationOptions, completionHandler: @escaping @Sendable (Bool, Error?) -> Void)
    func add(_ request: UNNotificationRequest, withCompletionHandler completionHandler: (@Sendable (Error?) -> Void)?)
}

extension UNUserNotificationCenter: SystemNotificationCenter {
    func authorizationStatus(completion: @escaping (UNAuthorizationStatus) -> Void) {
        getNotificationSettings { completion($0.authorizationStatus) }
    }
}

/// Uses the Nexus app identity for due reminders. Ordinary action feedback stays in-app.
final class SystemNotificationPresenter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = SystemNotificationPresenter(center: UNUserNotificationCenter.current()) {
        NotificationPresenter.shared.deliver($0)
    }

    private let center: SystemNotificationCenter
    private let fallback: (PluginNotification) -> Void

    init(center: SystemNotificationCenter, fallback: @escaping (PluginNotification) -> Void) {
        self.center = center
        self.fallback = fallback
        super.init()
        center.delegate = self
    }

    func requestAuthorization(willPrompt: @escaping () -> Void, completion: @escaping (Bool) -> Void) {
        center.authorizationStatus { [weak self] status in
            DispatchQueue.main.async {
                guard let self else { return }
                switch status {
                case .authorized, .provisional:
                    completion(true)
                case .notDetermined:
                    willPrompt()
                    self.center.requestAuthorization(options: [.alert, .sound]) { granted, error in
                        if let error { NSLog("Nexus notification authorization failed: %@", error.localizedDescription) }
                        DispatchQueue.main.async { completion(granted && error == nil) }
                    }
                default:
                    completion(false)
                }
            }
        }
    }

    func deliver(_ notification: PluginNotification) {
        // Do not interrupt the user with an authorization dialog when a reminder is due.
        // Recheck every time because permission can change while Nexus is running.
        center.authorizationStatus { [weak self] status in
            guard let self else { return }
            guard status == .authorized || status == .provisional else {
                self.deliverFallback(notification)
                return
            }
            let content = UNMutableNotificationContent()
            content.title = notification.title
            content.body = notification.body
            content.sound = .default
            content.threadIdentifier = "nexus.reminders"
            let request = UNNotificationRequest(
                identifier: "io.clipkeep.reminders:\(notification.id)",
                content: content,
                trigger: nil
            )
            self.center.add(request) { [weak self] error in
                guard let error else { return }
                NSLog("Nexus reminder notification failed: %@", error.localizedDescription)
                self?.deliverFallback(notification)
            }
        }
    }

    private func deliverFallback(_ notification: PluginNotification) {
        DispatchQueue.main.async { [weak self] in self?.fallback(notification) }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }
}
