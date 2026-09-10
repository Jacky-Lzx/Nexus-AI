import AppKit
import ApplicationServices

struct MenuBarSnapshotItem: Codable {
    let id: String
    let title: String
    let subtitle: String
    let index: Int
}

final class MenuBarProvider {
    func items(for application: NSRunningApplication) -> [MenuBarSnapshotItem] {
        guard let menuBar = menuBar(for: application.processIdentifier),
              let children = attribute(kAXChildrenAttribute, from: menuBar) as? [AXUIElement]
        else { return [] }

        return children.enumerated().compactMap { index, item in
            let rawTitle = attribute(kAXTitleAttribute, from: item) as? String ?? ""
            let description = attribute(kAXDescriptionAttribute, from: item) as? String ?? ""
            let title = rawTitle.isEmpty && index == 0 ? "Apple" : rawTitle
            guard !title.isEmpty else { return nil }
            return MenuBarSnapshotItem(
                id: "\(application.processIdentifier):\(index)",
                title: title,
                subtitle: application.localizedName ?? description,
                index: index
            )
        }
    }

    func pressItem(processIdentifier: pid_t, index: Int) -> Bool {
        guard let menuBar = menuBar(for: processIdentifier),
              let children = attribute(kAXChildrenAttribute, from: menuBar) as? [AXUIElement],
              children.indices.contains(index)
        else { return false }
        return AXUIElementPerformAction(children[index], kAXPressAction as CFString) == .success
    }

    private func menuBar(for processIdentifier: pid_t) -> AXUIElement? {
        let application = AXUIElementCreateApplication(processIdentifier)
        return attribute(kAXMenuBarAttribute, from: application) as! AXUIElement?
    }

    private func attribute(_ name: String, from element: AXUIElement) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
            return nil
        }
        return value
    }
}
