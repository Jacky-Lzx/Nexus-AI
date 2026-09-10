import AppKit
import ApplicationServices

struct MenuBarSnapshotItem: Codable {
    let id: String
    let title: String
    let subtitle: String
    let path: String
    let pathComponents: [String]

    enum CodingKeys: String, CodingKey {
        case id, title, subtitle, path
        case pathComponents = "path_components"
    }
}

enum MenuBarPressResult {
    case success
    case menuNotFound
    case actionFailed(AXError)
}

final class MenuBarProvider {
    func items(for application: NSRunningApplication) -> [MenuBarSnapshotItem] {
        guard let menuBar = menuBar(for: application.processIdentifier),
              let children = attribute(kAXChildrenAttribute, from: menuBar) as? [AXUIElement]
        else { return [] }

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

    func pressItem(processIdentifier: pid_t, path: [String]) -> MenuBarPressResult {
        guard let topLevelTitle = path.first,
              let menuBar = menuBar(for: processIdentifier),
              let topLevelItem = descendants(of: menuBar).enumerated().first(where: { index, item in
                  normalizedTopLevelTitle(of: item, index: index) == topLevelTitle
              })?.element,
              var menu = descendants(of: topLevelItem).first(where: { role(of: $0) == kAXMenuRole })
        else { return .menuNotFound }

        for (offset, title) in path.dropFirst().enumerated() {
            guard let item = descendants(of: menu).first(where: {
                role(of: $0) == kAXMenuItemRole
                    && (attribute(kAXTitleAttribute, from: $0) as? String) == title
            }) else { return .menuNotFound }
            if offset == path.count - 2 {
                let error = AXUIElementPerformAction(item, kAXPressAction as CFString)
                return error == .success ? .success : .actionFailed(error)
            }
            guard let submenu = descendants(of: item).first(where: { role(of: $0) == kAXMenuRole })
            else { return .menuNotFound }
            menu = submenu
        }
        return .menuNotFound
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
                items.append(MenuBarSnapshotItem(
                    id: id,
                    title: title,
                    subtitle: applicationName,
                    path: path.joined(separator: " › "),
                    pathComponents: path
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

    private func normalizedTopLevelTitle(of element: AXUIElement, index: Int) -> String {
        let title = attribute(kAXTitleAttribute, from: element) as? String ?? ""
        return title.isEmpty && index == 0 ? "Apple" : title
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
