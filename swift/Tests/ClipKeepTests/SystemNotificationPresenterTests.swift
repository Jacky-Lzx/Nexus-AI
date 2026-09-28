import XCTest
import UserNotifications
@testable import Nexus

private final class FakeNotificationCenter: SystemNotificationCenter {
    weak var delegate: UNUserNotificationCenterDelegate?
    var status: UNAuthorizationStatus = .authorized
    var requests: [UNNotificationRequest] = []
    var requestedOptions: UNAuthorizationOptions?
    var submissionError: Error?
    var permissionGranted = true

    func authorizationStatus(completion: @escaping (UNAuthorizationStatus) -> Void) {
        completion(status)
    }

    func requestAuthorization(options: UNAuthorizationOptions, completionHandler: @escaping @Sendable (Bool, Error?) -> Void) {
        requestedOptions = options
        completionHandler(permissionGranted, nil)
    }

    func add(_ request: UNNotificationRequest, withCompletionHandler completionHandler: (@Sendable (Error?) -> Void)?) {
        requests.append(request)
        completionHandler?(submissionError)
    }
}

final class SystemNotificationPresenterTests: XCTestCase {
    private let reminder = PluginNotification(id: "reminder:42", title: "Reminder", body: "Take a break")

    func testAuthorizedReminderSubmitsNativeContentWithoutToast() {
        let center = FakeNotificationCenter()
        let presenter = SystemNotificationPresenter(center: center) { _ in XCTFail("Unexpected toast") }
        presenter.deliver(reminder)
        XCTAssertTrue(center.delegate === presenter)
        XCTAssertEqual(center.requests.count, 1)
        XCTAssertEqual(center.requests.first?.identifier, "io.clipkeep.reminders:reminder:42")
        XCTAssertEqual(center.requests.first?.content.title, reminder.title)
        XCTAssertEqual(center.requests.first?.content.body, reminder.body)
        XCTAssertNotNil(center.requests.first?.content.sound)
        XCTAssertNil(center.requests.first?.trigger)
        XCTAssertNil(center.requestedOptions)
    }

    func testDeniedOrUndeterminedPermissionFallsBackWithoutPrompting() {
        for status: UNAuthorizationStatus in [.denied, .notDetermined] {
            let center = FakeNotificationCenter()
            center.status = status
            let delivered = expectation(description: "Fallback")
            let presenter = SystemNotificationPresenter(center: center) { notification in
                XCTAssertTrue(Thread.isMainThread)
                XCTAssertEqual(notification.id, self.reminder.id)
                delivered.fulfill()
            }
            presenter.deliver(reminder)
            wait(for: [delivered], timeout: 2)
            XCTAssertTrue(center.requests.isEmpty)
            XCTAssertNil(center.requestedOptions)
            withExtendedLifetime(presenter) {}
        }
    }

    func testSubmissionFailureFallsBackExactlyOnce() {
        let center = FakeNotificationCenter()
        center.submissionError = NSError(domain: "test", code: 1)
        let delivered = expectation(description: "Fallback after failure")
        delivered.assertForOverFulfill = true
        let presenter = SystemNotificationPresenter(center: center) { _ in delivered.fulfill() }
        presenter.deliver(reminder)
        wait(for: [delivered], timeout: 2)
        XCTAssertEqual(center.requests.count, 1)
        withExtendedLifetime(presenter) {}
    }

    func testFirstAuthorizationPromptsBeforeRequestingAlertAndSound() {
        let center = FakeNotificationCenter()
        center.status = .notDetermined
        let presenter = SystemNotificationPresenter(center: center) { _ in XCTFail("Unexpected toast") }
        let completed = expectation(description: "Permission result")
        var willPrompt = false
        presenter.requestAuthorization(willPrompt: {
            XCTAssertTrue(Thread.isMainThread)
            XCTAssertNil(center.requestedOptions)
            willPrompt = true
        }) { granted in
            XCTAssertTrue(Thread.isMainThread)
            XCTAssertTrue(willPrompt)
            XCTAssertTrue(granted)
            XCTAssertEqual(center.requestedOptions, [.alert, .sound])
            completed.fulfill()
        }
        wait(for: [completed], timeout: 2)
        withExtendedLifetime(presenter) {}
    }

    func testExistingPermissionDoesNotPromptAgain() {
        for status: UNAuthorizationStatus in [.authorized, .provisional, .denied] {
            let center = FakeNotificationCenter()
            center.status = status
            let presenter = SystemNotificationPresenter(center: center) { _ in XCTFail("Unexpected toast") }
            let completed = expectation(description: "Existing permission")
            presenter.requestAuthorization(willPrompt: { XCTFail("Unexpected prompt") }) { granted in
                XCTAssertEqual(granted, status != .denied)
                completed.fulfill()
            }
            wait(for: [completed], timeout: 2)
            XCTAssertNil(center.requestedOptions)
            withExtendedLifetime(presenter) {}
        }
    }
}
