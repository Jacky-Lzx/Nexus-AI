import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var coordinator: SearchCoordinator!
    private var launcher: LauncherPanelController!
    private var statusItem: NSStatusItem!
    private var settingsWindow: SettingsWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        coordinator = SearchCoordinator()
        launcher = LauncherPanelController(coordinator: coordinator)
        launcher.onPluginStateChanged = { [weak self] in
            self?.refreshHotKeys()
            self?.settingsWindow?.refresh()
        }
        configureStatusItem()

        GlobalHotKey.shared.handler = { [weak self] action in
            guard let self, self.coordinator.isEnabled(action.pluginId) else { return }
            switch action {
            case .applications: self.launcher.toggle(mode: .applications)
            case .clipboard: self.launcher.toggle(mode: .clipboard)
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
            withTitle: "Search Applications  \(GlobalHotKey.Action.applications.shortcut.displayName)",
            action: #selector(showApplications),
            keyEquivalent: ""
        )
        applications.target = self
        applications.isEnabled = coordinator.isEnabled(GlobalHotKey.Action.applications.pluginId)
        let clipboard = menu.addItem(
            withTitle: "Search Clipboard  \(GlobalHotKey.Action.clipboard.shortcut.displayName)",
            action: #selector(showClipboard),
            keyEquivalent: ""
        )
        clipboard.target = self
        clipboard.isEnabled = coordinator.isEnabled(GlobalHotKey.Action.clipboard.pluginId)
        menu.addItem(.separator())
        let settings = NSMenuItem(title: "Settings…", action: #selector(showSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)
        menu.addItem(.separator())

        let plugins = NSMenu()
        for descriptor in builtInPlugins {
            let item = NSMenuItem(title: descriptor.name, action: #selector(togglePlugin(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = descriptor.id
            item.state = coordinator.isEnabled(descriptor.id) ? .on : .off
            plugins.addItem(item)
        }
        let pluginsItem = NSMenuItem(title: "Plugins", action: nil, keyEquivalent: "")
        pluginsItem.submenu = plugins
        menu.addItem(pluginsItem)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit ClipKeep", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
        statusItem.menu = menu
    }

    @objc private func showApplications() {
        guard coordinator.isEnabled(GlobalHotKey.Action.applications.pluginId) else { return }
        launcher.show(mode: .applications)
    }

    @objc private func showClipboard() {
        guard coordinator.isEnabled(GlobalHotKey.Action.clipboard.pluginId) else { return }
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
              let descriptor = builtInPlugins.first(where: { $0.id == id }) else { return }
        coordinator.setEnabled(!coordinator.isEnabled(id), descriptor: descriptor)
        settingsWindow?.refresh()
    }

    @objc private func quit() { NSApp.terminate(nil) }

    private func refreshHotKeys() {
        let enabledActions = Set(GlobalHotKey.Action.allCases.filter {
            coordinator.isEnabled($0.pluginId)
        })
        let failures = GlobalHotKey.shared.configure(enabledActions: enabledActions)
        if !failures.isEmpty {
            coordinator.onStatusChanged?(
                "Could not register: \(failures.map { $0.shortcut.displayName }.joined(separator: ", "))"
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
