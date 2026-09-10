import AppKit
import ApplicationServices

struct MenuBarSnapshotItem: Codable {
    let id: String
    let title: String
    let subtitle: String
    let path: String
}

final class MenuBarProvider {
    private var targetProcessIdentifier: pid_t?
    private var elementsByID: [String: AXUIElement] = [:]

    func items(for application: NSRunningApplication) -> [MenuBarSnapshotItem] {
        guard let menuBar = menuBar(for: application.processIdentifier),
              let children = attribute(kAXChildrenAttribute, from: menuBar) as? [AXUIElement]
        else { return [] }

        targetProcessIdentifier = application.processIdentifier
        elementsByID.removeAll(keepingCapacity: true)
        var items: [MenuBarSnapshotItem] = []
        var nextID = 0
        for (index, item) in children.enumerated() {
            let rawTitle = attribute(kAXTitleAttribute, from: item) as? String ?? ""
            let title = rawTitle.isEmpty && index == 0 ? "Apple" : rawTitle
            guard !title.isEmpty,
                  let menu = descendants(of: item).first(where: { role(of: $0) == kAXMenuRole })
            else { continue }
            collectItems(
                from: menu,
                parentPath: [title],
                applicationName: application.localizedName ?? "Application",
                processIdentifier: application.processIdentifier,
                nextID: &nextID,
                into: &items
            )
        }
        return items
    }

    func pressItem(processIdentifier: pid_t, id: String) -> Bool {
        guard targetProcessIdentifier == processIdentifier, let element = elementsByID[id] else {
            return false
        }
        return AXUIElementPerformAction(element, kAXPressAction as CFString) == .success
    }

    private func collectItems(
        from menu: AXUIElement,
        parentPath: [String],
        applicationName: String,
        processIdentifier: pid_t,
        nextID: inout Int,
        into items: inout [MenuBarSnapshotItem]
    ) {
        for element in descendants(of: menu) where role(of: element) == kAXMenuItemRole {
            let title = (attribute(kAXTitleAttribute, from: element) as? String ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard !title.isEmpty else { continue }
            let path = parentPath + [title]
            let submenu = descendants(of: element).first(where: { role(of: $0) == kAXMenuRole })
            if let submenu {
                collectItems(
                    from: submenu,
                    parentPath: path,
                    applicationName: applicationName,
                    processIdentifier: processIdentifier,
                    nextID: &nextID,
                    into: &items
                )
            } else if (attribute(kAXEnabledAttribute, from: element) as? Bool) != false {
                let id = "\(processIdentifier):\(nextID)"
                nextID += 1
                elementsByID[id] = element
                items.append(MenuBarSnapshotItem(
                    id: id,
                    title: title,
                    subtitle: applicationName,
                    path: path.joined(separator: " › ")
                ))
            }
        }
    }

    private func descendants(of element: AXUIElement) -> [AXUIElement] {
        attribute(kAXChildrenAttribute, from: element) as? [AXUIElement] ?? []
    }

    private func role(of element: AXUIElement) -> String {
        attribute(kAXRoleAttribute, from: element) as? String ?? ""
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
