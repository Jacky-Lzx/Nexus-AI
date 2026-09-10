import Foundation
import UserNotifications

final class NotificationPresenter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationPresenter()

    private override init() {
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    func requestAuthorization(
        willPrompt: @escaping () -> Void,
        completion: @escaping (Bool) -> Void
    ) {
        let center = UNUserNotificationCenter.current()
        center.getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral:
                completion(true)
            case .notDetermined:
                DispatchQueue.main.async {
                    willPrompt()
                    center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
                        completion(granted)
                    }
                }
            default:
                completion(false)
            }
        }
    }

    func deliver(_ notification: PluginNotification) {
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.body
        content.sound = .default
        let request = UNNotificationRequest(
            identifier: notification.id,
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound])
    }
}
