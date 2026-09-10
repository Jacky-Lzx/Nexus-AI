import AppKit
import Carbon

private enum SettingsLayout {
    static let windowSize = NSSize(width: 900, height: 700)
    static let sidebarWidth: CGFloat = 240
}

private struct SettingsPaneItem {
    let id: String
    let name: String
    let symbolName: String
    let plugin: PluginDescriptor?

}

final class SettingsWindowController: NSWindowController,
    NSSearchFieldDelegate,
    NSTableViewDataSource,
    NSTableViewDelegate {

    private let coordinator: SearchCoordinator
    private unowned let launcher: LauncherPanelController
    private let onShortcutChanged: () -> Void
    private let root = NSView()
    private let sidebar = NSView()
    private let searchField = NSSearchField()
    private let tableView = NSTableView()
    private let detailPane = NSView()
    private let allPanes: [SettingsPaneItem]
    private var visiblePanes: [SettingsPaneItem]

    init(
        coordinator: SearchCoordinator,
        launcher: LauncherPanelController,
        onShortcutChanged: @escaping () -> Void
    ) {
        self.coordinator = coordinator
        self.launcher = launcher
        self.onShortcutChanged = onShortcutChanged
        allPanes = [SettingsPaneItem(
            id: "key-mappings", name: "Key Mappings", symbolName: "keyboard", plugin: nil
        )] + coordinator.descriptors.map { plugin in
            SettingsPaneItem(
                id: plugin.id,
                name: plugin.name,
                symbolName: plugin.symbolName,
                plugin: plugin
            )
        }
        visiblePanes = allPanes
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: SettingsLayout.windowSize),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        super.init(window: window)
        configure(window)
        buildInterface()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func refresh() {
        let selectedId = selectedPane?.id
        tableView.reloadData()
        let row = selectedId.flatMap { id in
            visiblePanes.firstIndex(where: { $0.id == id })
        } ?? (visiblePanes.isEmpty ? nil : 0)
        if let row {
            tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        showSelectedPlugin()
    }

    private func configure(_ window: NSWindow) {
        window.title = "Nexus Settings"
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 760, height: 500)
        window.center()
    }

    private func buildInterface() {
        guard let window else { return }
        root.wantsLayer = true

        let glass = NSGlassEffectView(frame: window.contentView?.bounds ?? .zero)
        glass.autoresizingMask = [.width, .height]
        glass.style = .regular
        glass.tintColor = NSColor.windowBackgroundColor.withAlphaComponent(0.08)
        glass.contentView = root
        window.contentView = glass

        let splitView = NSSplitView()
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(splitView)

        sidebar.wantsLayer = true
        sidebar.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.10).cgColor
        detailPane.wantsLayer = true
        splitView.addArrangedSubview(sidebar)
        splitView.addArrangedSubview(detailPane)
        sidebar.widthAnchor.constraint(equalToConstant: SettingsLayout.sidebarWidth).isActive = true

        searchField.placeholderString = "Search settings…"
        searchField.delegate = self
        searchField.translatesAutoresizingMaskIntoConstraints = false
        sidebar.addSubview(searchField)

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("plugin"))
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 48
        tableView.intercellSpacing = NSSize(width: 0, height: 4)
        tableView.backgroundColor = .clear
        tableView.selectionHighlightStyle = .none
        tableView.dataSource = self
        tableView.delegate = self

        let scrollView = NSScrollView()
        scrollView.documentView = tableView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        sidebar.addSubview(scrollView)

        NSLayoutConstraint.activate([
            splitView.topAnchor.constraint(equalTo: root.topAnchor),
            splitView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            splitView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            searchField.topAnchor.constraint(equalTo: sidebar.topAnchor, constant: 54),
            searchField.leadingAnchor.constraint(equalTo: sidebar.leadingAnchor, constant: 16),
            searchField.trailingAnchor.constraint(equalTo: sidebar.trailingAnchor, constant: -16),
            searchField.heightAnchor.constraint(equalToConstant: 34),
            scrollView.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 14),
            scrollView.leadingAnchor.constraint(equalTo: sidebar.leadingAnchor, constant: 10),
            scrollView.trailingAnchor.constraint(equalTo: sidebar.trailingAnchor, constant: -10),
            scrollView.bottomAnchor.constraint(equalTo: sidebar.bottomAnchor, constant: -12),
        ])

        tableView.reloadData()
        tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        showSelectedPlugin()
    }

    func numberOfRows(in tableView: NSTableView) -> Int { visiblePanes.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("PluginSettingsCell")
        let cell = tableView.makeView(withIdentifier: identifier, owner: self) as? PluginSettingsCell
            ?? PluginSettingsCell()
        cell.identifier = identifier
        cell.configure(visiblePanes[row], selected: row == tableView.selectedRow)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        tableView.reloadData(
            forRowIndexes: IndexSet(integersIn: 0..<visiblePanes.count),
            columnIndexes: IndexSet(integer: 0)
        )
        showSelectedPlugin()
    }

    func controlTextDidChange(_ obj: Notification) {
        let query = searchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        visiblePanes = query.isEmpty
            ? allPanes
            : allPanes.filter { $0.name.localizedCaseInsensitiveContains(query) }
        tableView.reloadData()
        if !visiblePanes.isEmpty {
            tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }
        showSelectedPlugin()
    }

    private var selectedPane: SettingsPaneItem? {
        guard visiblePanes.indices.contains(tableView.selectedRow) else { return nil }
        return visiblePanes[tableView.selectedRow]
    }

    private var selectedPlugin: PluginDescriptor? {
        selectedPane?.plugin
    }

    private func showSelectedPlugin() {
        detailPane.subviews.forEach { $0.removeFromSuperview() }
        guard let pane = selectedPane else {
            let empty = NSTextField(labelWithString: "No settings found")
            empty.textColor = .secondaryLabelColor
            empty.translatesAutoresizingMaskIntoConstraints = false
            detailPane.addSubview(empty)
            NSLayoutConstraint.activate([
                empty.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
                empty.centerYAnchor.constraint(equalTo: detailPane.centerYAnchor),
            ])
            return
        }
        guard let plugin = pane.plugin else {
            showKeyMappings()
            return
        }

        let icon = NSImageView()
        icon.image = NSImage(
            systemSymbolName: plugin.symbolName,
            accessibilityDescription: plugin.name
        )
        icon.contentTintColor = .labelColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 44, weight: .medium)
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: plugin.name)
        title.font = .systemFont(ofSize: 28, weight: .bold)
        title.translatesAutoresizingMaskIntoConstraints = false

        let pluginDescription: String
        switch plugin.id {
        case "io.clipkeep.clipboard":
            pluginDescription = "Search, preview, and paste your clipboard history"
        case "io.clipkeep.applications":
            pluginDescription = "Quickly search and open applications installed on this Mac"
        case "io.clipkeep.calculator":
            pluginDescription = "Evaluate expressions and paste the result"
        case "io.clipkeep.calendar":
            pluginDescription = "Read-only access to your upcoming calendar events"
        case "io.clipkeep.kill-process":
            pluginDescription = "Find running processes and terminate them after confirmation"
        case "io.clipkeep.system-commands":
            pluginDescription = "Quickly run common macOS system actions"
        default:
            pluginDescription = "Rust plugin \(plugin.id) · version \(plugin.version)"
        }
        let description = NSTextField(labelWithString: pluginDescription)
        description.font = .systemFont(ofSize: 14)
        description.textColor = .secondaryLabelColor
        description.translatesAutoresizingMaskIntoConstraints = false

        let enabled = NSSwitch()
        enabled.state = coordinator.isEnabled(plugin.id) ? .on : .off
        enabled.target = self
        enabled.action = #selector(toggleSelectedPlugin(_:))
        enabled.translatesAutoresizingMaskIntoConstraints = false

        let optionsTitle = NSTextField(labelWithString: "Options")
        optionsTitle.font = .systemFont(ofSize: 13, weight: .semibold)
        optionsTitle.textColor = .secondaryLabelColor
        optionsTitle.translatesAutoresizingMaskIntoConstraints = false

        let options = NSStackView()
        options.orientation = .vertical
        options.spacing = 0
        options.wantsLayer = true
        options.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.055).cgColor
        options.layer?.cornerRadius = 16
        options.layer?.cornerCurve = .continuous
        options.translatesAutoresizingMaskIntoConstraints = false

        let shortcut = ShortcutRecorderButton(
            shortcut: { GlobalHotKey.shortcut(for: plugin.id) },
            onChange: { [weak self] shortcut in
                GlobalHotKey.save(shortcut, for: plugin.id)
                self?.onShortcutChanged()
            }
        )
        options.addArrangedSubview(settingRow(
            title: "Keyboard Shortcut",
            description: "Click and press a shortcut to open this plugin",
            control: shortcut
        ))

        if plugin.id != "io.clipkeep.clipboard" {
            let includeInGlobalSearch = NSSwitch()
            includeInGlobalSearch.state = coordinator.isIncludedInGlobalSearch(plugin.id) ? .on : .off
            includeInGlobalSearch.identifier = NSUserInterfaceItemIdentifier(plugin.id)
            includeInGlobalSearch.target = self
            includeInGlobalSearch.action = #selector(changeGlobalSearchInclusion(_:))
            options.addArrangedSubview(settingRow(
                title: "Include in Global Search",
                description: "Mix this plugin's results into the main Nexus search",
                control: includeInGlobalSearch
            ))
        }

        if plugin.id == "io.clipkeep.clipboard" {
            let quality = NSPopUpButton()
            quality.addItems(withTitles: ImagePreviewQuality.allCases.map(\.title))
            quality.selectItem(withTitle: launcher.imagePreviewQuality.title)
            quality.target = self
            quality.action = #selector(changePreviewQuality(_:))
            options.addArrangedSubview(settingRow(
                title: "Image Preview Quality",
                description: "Choose a fast high-resolution preview or the original image",
                control: quality
            ))

            let retention = NSPopUpButton()
            let retentionChoices = [(1, "24 Hours"), (7, "7 Days"), (30, "30 Days"), (90, "3 Months")]
            retention.addItems(withTitles: retentionChoices.map(\.1))
            let currentDays = UserDefaults.standard.integer(forKey: ClipboardPreference.retentionDays)
            retention.selectItem(at: retentionChoices.firstIndex(where: { $0.0 == currentDays }) ?? 2)
            retention.tag = retentionChoices[retention.indexOfSelectedItem].0
            retention.target = self
            retention.action = #selector(changeClipboardRetention(_:))
            options.addArrangedSubview(settingRow(
                title: "Keep History",
                description: "Favorites are kept until you delete them",
                control: retention
            ))

            let concealed = NSSwitch()
            concealed.state = UserDefaults.standard.bool(forKey: ClipboardPreference.ignoreConcealed) ? .on : .off
            concealed.target = self
            concealed.action = #selector(changeIgnoreConcealed(_:))
            options.addArrangedSubview(settingRow(
                title: "Ignore Sensitive Content",
                description: "Do not save concealed or transient pasteboard data",
                control: concealed
            ))

            let ignoredApps = NSTextField(string: UserDefaults.standard.string(
                forKey: ClipboardPreference.ignoredBundleIds
            ) ?? "")
            ignoredApps.placeholderString = "com.example.password-manager"
            ignoredApps.identifier = NSUserInterfaceItemIdentifier("ignoredBundleIds")
            ignoredApps.target = self
            ignoredApps.action = #selector(changeIgnoredApplications(_:))
            ignoredApps.widthAnchor.constraint(equalToConstant: 245).isActive = true
            options.addArrangedSubview(settingRow(
                title: "Ignored Applications",
                description: "Comma-separated bundle identifiers; press Return to save",
                control: ignoredApps
            ))

            let clear = NSPopUpButton()
            clear.addItems(withTitles: ["Clear…", "Last 5 Minutes", "Last 15 Minutes", "All History"])
            clear.target = self
            clear.action = #selector(clearClipboardHistory(_:))
            options.addArrangedSubview(settingRow(
                title: "Clear History",
                description: "Favorites are not removed",
                control: clear
            ))
        } else if plugin.id == "io.clipkeep.applications" {
            let locations = NSTextField(labelWithString: "/Applications and /System/Applications")
            locations.textColor = .secondaryLabelColor
            locations.lineBreakMode = .byTruncatingMiddle
            options.addArrangedSubview(settingRow(
                title: "Indexed Locations",
                description: "Application results refresh automatically every 60 seconds",
                control: locations
            ))
        } else if plugin.id == "io.clipkeep.calendar" {
            let days = NSPopUpButton()
            let choices = [1, 7, 14, 30, 90]
            days.addItems(withTitles: choices.map { $0 == 1 ? "Next 24 Hours" : "Next \($0) Days" })
            let selectedDays = UserDefaults.standard.integer(forKey: CalendarPreference.futureDays)
            days.selectItem(at: choices.firstIndex(of: selectedDays) ?? 2)
            days.target = self
            days.action = #selector(changeCalendarRange(_:))
            options.addArrangedSubview(settingRow(
                title: "Event Range",
                description: "How far ahead to include events",
                control: days
            ))
        } else {
            let capabilities = NSTextField(labelWithString: plugin.capabilities.joined(separator: ", "))
            capabilities.textColor = .secondaryLabelColor
            capabilities.lineBreakMode = .byTruncatingTail
            options.addArrangedSubview(settingRow(
                title: "Capabilities",
                description: "Permissions declared by plugin.toml",
                control: capabilities
            ))
        }

        for view in [icon, title, description, enabled, optionsTitle, options] {
            detailPane.addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 76),
            icon.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
            icon.widthAnchor.constraint(equalToConstant: 64),
            icon.heightAnchor.constraint(equalToConstant: 64),
            title.topAnchor.constraint(equalTo: icon.bottomAnchor, constant: 14),
            title.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
            description.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 8),
            description.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
            description.leadingAnchor.constraint(greaterThanOrEqualTo: detailPane.leadingAnchor, constant: 28),
            enabled.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 22),
            enabled.trailingAnchor.constraint(equalTo: detailPane.trailingAnchor, constant: -24),
            optionsTitle.topAnchor.constraint(equalTo: description.bottomAnchor, constant: 30),
            optionsTitle.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 30),
            options.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 24),
            options.trailingAnchor.constraint(equalTo: detailPane.trailingAnchor, constant: -24),
            options.topAnchor.constraint(equalTo: optionsTitle.bottomAnchor, constant: 10),
        ])
    }

    private func showKeyMappings() {
        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: "keyboard", accessibilityDescription: "Key Mappings")
        icon.contentTintColor = .labelColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 44, weight: .medium)
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: "Key Mappings")
        title.font = .systemFont(ofSize: 28, weight: .bold)
        title.translatesAutoresizingMaskIntoConstraints = false

        let description = NSTextField(labelWithString: "Customize keyboard navigation in the launcher")
        description.font = .systemFont(ofSize: 14)
        description.textColor = .secondaryLabelColor
        description.translatesAutoresizingMaskIntoConstraints = false

        let optionsTitle = NSTextField(labelWithString: "Navigation")
        optionsTitle.font = .systemFont(ofSize: 13, weight: .semibold)
        optionsTitle.textColor = .secondaryLabelColor
        optionsTitle.translatesAutoresizingMaskIntoConstraints = false

        let options = NSStackView()
        options.orientation = .vertical
        options.spacing = 0
        options.wantsLayer = true
        options.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.055).cgColor
        options.layer?.cornerRadius = 16
        options.layer?.cornerCurve = .continuous
        options.translatesAutoresizingMaskIntoConstraints = false

        let globalShortcut = ShortcutRecorderButton(
            shortcut: { GlobalHotKey.shortcut(for: GlobalHotKey.globalSearchId) },
            onChange: { [weak self] shortcut in
                GlobalHotKey.save(shortcut, for: GlobalHotKey.globalSearchId)
                self?.onShortcutChanged()
            }
        )
        options.addArrangedSubview(settingRow(
            title: "Global Search",
            description: "Open the combined search across participating plugins",
            control: globalShortcut
        ))

        for action in LauncherKeyAction.allCases {
            let shortcut = ShortcutRecorderButton(
                shortcut: { action.shortcut },
                onChange: { shortcut in action.save(shortcut: shortcut) }
            )
            options.addArrangedSubview(settingRow(
                title: action.title,
                description: action == .moveDown
                    ? "Move the selection to the next result"
                    : "Move the selection to the previous result",
                control: shortcut
            ))
        }

        for view in [icon, title, description, optionsTitle, options] {
            detailPane.addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 76),
            icon.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
            icon.widthAnchor.constraint(equalToConstant: 64),
            icon.heightAnchor.constraint(equalToConstant: 64),
            title.topAnchor.constraint(equalTo: icon.bottomAnchor, constant: 14),
            title.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
            description.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 8),
            description.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
            optionsTitle.topAnchor.constraint(equalTo: description.bottomAnchor, constant: 64),
            optionsTitle.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 30),
            options.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 24),
            options.trailingAnchor.constraint(equalTo: detailPane.trailingAnchor, constant: -24),
            options.topAnchor.constraint(equalTo: optionsTitle.bottomAnchor, constant: 10),
        ])
    }

    private func settingRow(title: String, description: String, control: NSView) -> NSView {
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = .systemFont(ofSize: 15, weight: .medium)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        let descriptionLabel = NSTextField(labelWithString: description)
        descriptionLabel.font = .systemFont(ofSize: 12)
        descriptionLabel.textColor = .secondaryLabelColor
        descriptionLabel.lineBreakMode = .byTruncatingTail
        descriptionLabel.translatesAutoresizingMaskIntoConstraints = false
        control.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(titleLabel)
        row.addSubview(descriptionLabel)
        row.addSubview(control)
        NSLayoutConstraint.activate([
            row.heightAnchor.constraint(equalToConstant: 68),
            titleLabel.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: 20),
            titleLabel.topAnchor.constraint(equalTo: row.topAnchor, constant: 12),
            titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: control.leadingAnchor, constant: -16),
            descriptionLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            descriptionLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 5),
            descriptionLabel.trailingAnchor.constraint(lessThanOrEqualTo: control.leadingAnchor, constant: -16),
            control.trailingAnchor.constraint(equalTo: row.trailingAnchor, constant: -18),
            control.centerYAnchor.constraint(equalTo: row.centerYAnchor),
        ])
        return row
    }

    @objc private func toggleSelectedPlugin(_ sender: NSSwitch) {
        guard let plugin = selectedPlugin else { return }
        coordinator.setEnabled(sender.state == .on, descriptor: plugin)
        tableView.reloadData()
    }

    @objc private func changeGlobalSearchInclusion(_ sender: NSSwitch) {
        guard let id = sender.identifier?.rawValue,
              let plugin = coordinator.descriptors.first(where: { $0.id == id }) else { return }
        coordinator.setIncludedInGlobalSearch(sender.state == .on, descriptor: plugin)
    }

    @objc private func changePreviewQuality(_ sender: NSPopUpButton) {
        guard let title = sender.selectedItem?.title,
              let quality = ImagePreviewQuality.allCases.first(where: { $0.title == title }) else { return }
        launcher.imagePreviewQuality = quality
    }

    @objc private func changeClipboardRetention(_ sender: NSPopUpButton) {
        let days = [1, 7, 30, 90][max(0, sender.indexOfSelectedItem)]
        UserDefaults.standard.set(days, forKey: ClipboardPreference.retentionDays)
        coordinator.reloadSettings(for: "io.clipkeep.clipboard")
    }

    @objc private func changeIgnoreConcealed(_ sender: NSSwitch) {
        UserDefaults.standard.set(sender.state == .on, forKey: ClipboardPreference.ignoreConcealed)
        coordinator.reloadSettings(for: "io.clipkeep.clipboard")
    }

    @objc private func changeIgnoredApplications(_ sender: NSTextField) {
        UserDefaults.standard.set(sender.stringValue, forKey: ClipboardPreference.ignoredBundleIds)
        coordinator.reloadSettings(for: "io.clipkeep.clipboard")
    }

    @objc private func clearClipboardHistory(_ sender: NSPopUpButton) {
        let actions = [nil, "clear_5m", "clear_15m", "clear_all"]
        guard actions.indices.contains(sender.indexOfSelectedItem),
              let action = actions[sender.indexOfSelectedItem] else { return }
        sender.selectItem(at: 0)
        let alert = NSAlert()
        alert.messageText = "Clear Clipboard History?"
        alert.informativeText = "This removes the selected non-favorite entries and cannot be undone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Clear")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        coordinator.execute(pluginId: "io.clipkeep.clipboard", actionId: action)
    }

    @objc private func changeCalendarRange(_ sender: NSPopUpButton) {
        let choices = [1, 7, 14, 30, 90]
        guard choices.indices.contains(sender.indexOfSelectedItem) else { return }
        UserDefaults.standard.set(choices[sender.indexOfSelectedItem], forKey: CalendarPreference.futureDays)
        coordinator.reloadSettings(for: "io.clipkeep.calendar")
    }
}

private final class ShortcutRecorderButton: NSButton {
    private let shortcut: () -> PluginShortcut?
    private let onChange: (PluginShortcut) -> Void
    private var monitor: Any?
    private var isRecording = false

    init(
        shortcut: @escaping () -> PluginShortcut?,
        onChange: @escaping (PluginShortcut) -> Void
    ) {
        self.shortcut = shortcut
        self.onChange = onChange
        super.init(frame: .zero)
        bezelStyle = .rounded
        target = self
        self.action = #selector(beginRecording)
        updateTitle()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func beginRecording() {
        guard !isRecording else { return }
        isRecording = true
        title = "Type shortcut…"
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, self.isRecording else { return event }
            if event.keyCode == UInt16(kVK_Escape) {
                self.finishRecording()
                return nil
            }
            guard let shortcut = PluginShortcut(event: event) else {
                NSSound.beep()
                return nil
            }
            self.finishRecording(displaying: shortcut)
            self.onChange(shortcut)
            return nil
        }
    }

    private func finishRecording(displaying newShortcut: PluginShortcut? = nil) {
        isRecording = false
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
        title = newShortcut?.displayName ?? shortcut()?.displayName ?? "Record Shortcut"
    }

    private func updateTitle() {
        title = shortcut()?.displayName ?? "Record Shortcut"
    }

    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }
}

private final class PluginSettingsCell: NSTableCellView {
    private let icon = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.cornerRadius = 10
        layer?.cornerCurve = .continuous
        icon.contentTintColor = .labelColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 18, weight: .medium)
        icon.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.font = .systemFont(ofSize: 14, weight: .medium)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(icon)
        addSubview(titleLabel)
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 24),
            icon.heightAnchor.constraint(equalToConstant: 24),
            titleLabel.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            titleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ pane: SettingsPaneItem, selected: Bool) {
        icon.image = NSImage(
            systemSymbolName: pane.symbolName,
            accessibilityDescription: pane.name
        )
        titleLabel.stringValue = pane.name
        layer?.backgroundColor = selected
            ? NSColor.white.withAlphaComponent(0.11).cgColor
            : NSColor.clear.cgColor
    }
}
