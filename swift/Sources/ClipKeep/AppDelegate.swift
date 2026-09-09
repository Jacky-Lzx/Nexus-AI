import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var coordinator: SearchCoordinator!
    private var launcher: LauncherPanelController!
    private var statusItem: NSStatusItem!

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        coordinator = SearchCoordinator()
        launcher = LauncherPanelController(coordinator: coordinator)
        launcher.onPluginStateChanged = { [weak self] in self?.rebuildMenu() }
        configureStatusItem()

        GlobalHotKey.shared.handler = { [weak self] in self?.launcher.toggle() }
        if !GlobalHotKey.shared.register() {
            coordinator.onStatusChanged?("Could not register ⌘⇧Space")
        }
        launcher.show(query: initialQuery())
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        launcher.show()
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
        menu.addItem(withTitle: "Show ClipKeep", action: #selector(showLauncher), keyEquivalent: "")
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
        menu.items.first?.target = self
        statusItem.menu = menu
    }

    @objc private func showLauncher() { launcher.show() }

    @objc private func togglePlugin(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String,
              let descriptor = builtInPlugins.first(where: { $0.id == id }) else { return }
        coordinator.setEnabled(!coordinator.isEnabled(id), descriptor: descriptor)
    }

    @objc private func quit() { NSApp.terminate(nil) }

    private func initialQuery() -> String {
        let arguments = ProcessInfo.processInfo.arguments
        guard let index = arguments.firstIndex(of: "--query"), arguments.indices.contains(index + 1) else {
            return ""
        }
        return arguments[index + 1]
    }
}
