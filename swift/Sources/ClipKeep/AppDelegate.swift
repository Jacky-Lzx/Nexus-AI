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
        launcher.onPluginStateChanged = { [weak self] in self?.rebuildMenu() }
        configureStatusItem()

        GlobalHotKey.shared.handler = { [weak self] action in
            switch action {
            case .applications: self?.launcher.toggle(mode: .applications)
            case .clipboard: self?.launcher.toggle(mode: .clipboard)
            }
        }
        if !GlobalHotKey.shared.register() {
            coordinator.onStatusChanged?("Could not register global shortcuts")
        }
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
        menu.addItem(withTitle: "Search Applications  ⌘⇧Space", action: #selector(showApplications), keyEquivalent: "")
        menu.addItem(withTitle: "Search Clipboard  ⌘⇧C", action: #selector(showClipboard), keyEquivalent: "")
        menu.items[0].target = self
        menu.items[1].target = self
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

    @objc private func showApplications() { launcher.show(mode: .applications) }
    @objc private func showClipboard() { launcher.show(mode: .clipboard) }

    @objc private func showSettings() {
        if settingsWindow == nil {
            settingsWindow = SettingsWindowController(coordinator: coordinator, launcher: launcher)
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
