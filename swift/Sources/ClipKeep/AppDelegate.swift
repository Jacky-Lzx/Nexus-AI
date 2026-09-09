import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var coordinator: SearchCoordinator!
    private var launcher: LauncherPanelController!
    private var statusItem: NSStatusItem!
    private var settingsWindow: SettingsWindowController?
    private var plugins: [PluginDescriptor] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        plugins = PluginRegistry.discover()
        coordinator = SearchCoordinator(descriptors: plugins)
        launcher = LauncherPanelController(coordinator: coordinator)
        launcher.onPluginStateChanged = { [weak self] in
            self?.refreshHotKeys()
            self?.settingsWindow?.refresh()
        }
        configureStatusItem()

        GlobalHotKey.shared.handler = { [weak self] pluginId in
            guard let self, self.coordinator.isEnabled(pluginId),
                  let descriptor = self.plugins.first(where: { $0.id == pluginId }) else { return }
            switch pluginId {
            case "io.clipkeep.applications": self.launcher.toggle(mode: .applications)
            case "io.clipkeep.clipboard": self.launcher.toggle(mode: .clipboard)
            default: self.launcher.toggle(mode: .plugin(descriptor))
            }
        }
        refreshHotKeys()
        launcher.show(mode: initialMode(), query: initialQuery())
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        launcher.show(mode: .applications)
        return true
    }

    private func configureStatusItem() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "clipboard", accessibilityDescription: "ClipKeep")
            button.toolTip = "ClipKeep"
        }
        rebuildMenu()
    }

    private func rebuildMenu() {
        let menu = NSMenu()
        let applications = menu.addItem(
            withTitle: "Search Applications  \(GlobalHotKey.shortcut(for: "io.clipkeep.applications")?.displayName ?? "")",
            action: #selector(showApplications),
            keyEquivalent: ""
        )
        applications.target = self
        applications.isEnabled = coordinator.isEnabled("io.clipkeep.applications")
        let clipboard = menu.addItem(
            withTitle: "Search Clipboard  \(GlobalHotKey.shortcut(for: "io.clipkeep.clipboard")?.displayName ?? "")",
            action: #selector(showClipboard),
            keyEquivalent: ""
        )
        clipboard.target = self
        clipboard.isEnabled = coordinator.isEnabled("io.clipkeep.clipboard")
        menu.addItem(.separator())
        let settings = NSMenuItem(title: "Settings…", action: #selector(showSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)
        menu.addItem(.separator())

        let pluginsMenu = NSMenu()
        for descriptor in plugins {
            let submenu = NSMenu()
            let open = NSMenuItem(title: "Open", action: #selector(showPlugin(_:)), keyEquivalent: "")
            open.target = self
            open.representedObject = descriptor.id
            open.isEnabled = coordinator.isEnabled(descriptor.id)
            submenu.addItem(open)
            let enabled = NSMenuItem(title: "Enabled", action: #selector(togglePlugin(_:)), keyEquivalent: "")
            enabled.target = self
            enabled.representedObject = descriptor.id
            enabled.state = coordinator.isEnabled(descriptor.id) ? .on : .off
            submenu.addItem(enabled)
            let item = NSMenuItem(title: descriptor.name, action: nil, keyEquivalent: "")
            item.submenu = submenu
            pluginsMenu.addItem(item)
        }
        let pluginsItem = NSMenuItem(title: "Plugins", action: nil, keyEquivalent: "")
        pluginsItem.submenu = pluginsMenu
        menu.addItem(pluginsItem)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit ClipKeep", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
        statusItem.menu = menu
    }

    @objc private func showApplications() {
        guard coordinator.isEnabled("io.clipkeep.applications") else { return }
        launcher.show(mode: .applications)
    }

    @objc private func showClipboard() {
        guard coordinator.isEnabled("io.clipkeep.clipboard") else { return }
        launcher.show(mode: .clipboard)
    }

    @objc private func showSettings() {
        if settingsWindow == nil {
            settingsWindow = SettingsWindowController(
                coordinator: coordinator,
                launcher: launcher,
                onShortcutChanged: { [weak self] in self?.refreshHotKeys() }
            )
        }
        settingsWindow?.showWindow(nil)
        settingsWindow?.refresh()
        NSApp.activate(ignoringOtherApps: true)
        settingsWindow?.window?.makeKeyAndOrderFront(nil)
    }

    @objc private func togglePlugin(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String,
              let descriptor = plugins.first(where: { $0.id == id }) else { return }
        coordinator.setEnabled(!coordinator.isEnabled(id), descriptor: descriptor)
        settingsWindow?.refresh()
    }

    @objc private func showPlugin(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String,
              coordinator.isEnabled(id),
              let descriptor = plugins.first(where: { $0.id == id }) else { return }
        switch id {
        case "io.clipkeep.applications": launcher.show(mode: .applications)
        case "io.clipkeep.clipboard": launcher.show(mode: .clipboard)
        default: launcher.show(mode: .plugin(descriptor))
        }
    }

    @objc private func quit() { NSApp.terminate(nil) }

    private func refreshHotKeys() {
        let enabledPluginIds = plugins.map(\.id).filter { coordinator.isEnabled($0) }
        let failures = GlobalHotKey.shared.configure(enabledPluginIds: enabledPluginIds)
        if !failures.isEmpty {
            coordinator.onStatusChanged?(
                "Could not register shortcuts for: \(failures.joined(separator: ", "))"
            )
        }
        rebuildMenu()
    }

    private func initialQuery() -> String {
        let arguments = ProcessInfo.processInfo.arguments
        guard let index = arguments.firstIndex(of: "--query"), arguments.indices.contains(index + 1) else {
            return ""
        }
        return arguments[index + 1]
    }

    private func initialMode() -> SearchMode {
        ProcessInfo.processInfo.arguments.contains("--clipboard") ? .clipboard : .applications
    }
}
