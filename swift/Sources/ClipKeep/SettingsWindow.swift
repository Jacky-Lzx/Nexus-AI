import AppKit
import Carbon

private enum SettingsLayout {
    static let windowSize = NSSize(width: 900, height: 600)
    static let sidebarWidth: CGFloat = 240
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
    private var visiblePlugins = builtInPlugins

    init(
        coordinator: SearchCoordinator,
        launcher: LauncherPanelController,
        onShortcutChanged: @escaping () -> Void
    ) {
        self.coordinator = coordinator
        self.launcher = launcher
        self.onShortcutChanged = onShortcutChanged
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
        let selectedId = selectedPlugin?.id
        tableView.reloadData()
        let row = selectedId.flatMap { id in
            visiblePlugins.firstIndex(where: { $0.id == id })
        } ?? (visiblePlugins.isEmpty ? nil : 0)
        if let row {
            tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        showSelectedPlugin()
    }

    private func configure(_ window: NSWindow) {
        window.title = "ClipKeep Settings"
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

        searchField.placeholderString = "Search plugins…"
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

    func numberOfRows(in tableView: NSTableView) -> Int { visiblePlugins.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("PluginSettingsCell")
        let cell = tableView.makeView(withIdentifier: identifier, owner: self) as? PluginSettingsCell
            ?? PluginSettingsCell()
        cell.identifier = identifier
        cell.configure(visiblePlugins[row], selected: row == tableView.selectedRow)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        tableView.reloadData(
            forRowIndexes: IndexSet(integersIn: 0..<visiblePlugins.count),
            columnIndexes: IndexSet(integer: 0)
        )
        showSelectedPlugin()
    }

    func controlTextDidChange(_ obj: Notification) {
        let query = searchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        visiblePlugins = query.isEmpty
            ? builtInPlugins
            : builtInPlugins.filter { $0.name.localizedCaseInsensitiveContains(query) }
        tableView.reloadData()
        if !visiblePlugins.isEmpty {
            tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }
        showSelectedPlugin()
    }

    private var selectedPlugin: PluginDescriptor? {
        guard visiblePlugins.indices.contains(tableView.selectedRow) else { return nil }
        return visiblePlugins[tableView.selectedRow]
    }

    private func showSelectedPlugin() {
        detailPane.subviews.forEach { $0.removeFromSuperview() }
        guard let plugin = selectedPlugin else {
            let empty = NSTextField(labelWithString: "No plugins found")
            empty.textColor = .secondaryLabelColor
            empty.translatesAutoresizingMaskIntoConstraints = false
            detailPane.addSubview(empty)
            NSLayoutConstraint.activate([
                empty.centerXAnchor.constraint(equalTo: detailPane.centerXAnchor),
                empty.centerYAnchor.constraint(equalTo: detailPane.centerYAnchor),
            ])
            return
        }

        let icon = NSImageView()
        icon.image = NSImage(
            systemSymbolName: plugin.id == "io.clipkeep.clipboard" ? "clipboard" : "square.grid.2x2",
            accessibilityDescription: plugin.name
        )
        icon.contentTintColor = .labelColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 44, weight: .medium)
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: plugin.name)
        title.font = .systemFont(ofSize: 28, weight: .bold)
        title.translatesAutoresizingMaskIntoConstraints = false

        let description = NSTextField(labelWithString: plugin.id == "io.clipkeep.clipboard"
            ? "Search, preview, and paste your clipboard history"
            : "Quickly search and open applications installed on this Mac")
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

        if let action = GlobalHotKey.Action(pluginId: plugin.id) {
            let shortcut = ShortcutRecorderButton(action: action) { [weak self] in
                self?.onShortcutChanged()
            }
            options.addArrangedSubview(settingRow(
                title: "Keyboard Shortcut",
                description: "Click and press a shortcut to open this plugin",
                control: shortcut
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
        } else {
            let locations = NSTextField(labelWithString: "/Applications and /System/Applications")
            locations.textColor = .secondaryLabelColor
            locations.lineBreakMode = .byTruncatingMiddle
            options.addArrangedSubview(settingRow(
                title: "Indexed Locations",
                description: "Application results refresh automatically every 60 seconds",
                control: locations
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
            row.heightAnchor.constraint(equalToConstant: 76),
            titleLabel.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: 20),
            titleLabel.topAnchor.constraint(equalTo: row.topAnchor, constant: 16),
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

    @objc private func changePreviewQuality(_ sender: NSPopUpButton) {
        guard let title = sender.selectedItem?.title,
              let quality = ImagePreviewQuality.allCases.first(where: { $0.title == title }) else { return }
        launcher.imagePreviewQuality = quality
    }
}

private final class ShortcutRecorderButton: NSButton {
    private let hotKeyAction: GlobalHotKey.Action
    private let onChange: () -> Void
    private var monitor: Any?
    private var isRecording = false

    init(action: GlobalHotKey.Action, onChange: @escaping () -> Void) {
        hotKeyAction = action
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
            self.hotKeyAction.save(shortcut: shortcut)
            self.finishRecording()
            self.onChange()
            return nil
        }
    }

    private func finishRecording() {
        isRecording = false
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
        updateTitle()
    }

    private func updateTitle() {
        title = hotKeyAction.shortcut.displayName
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

    func configure(_ plugin: PluginDescriptor, selected: Bool) {
        icon.image = NSImage(
            systemSymbolName: plugin.id == "io.clipkeep.clipboard" ? "clipboard" : "square.grid.2x2",
            accessibilityDescription: plugin.name
        )
        titleLabel.stringValue = plugin.name
        layer?.backgroundColor = selected
            ? NSColor.white.withAlphaComponent(0.11).cgColor
            : NSColor.clear.cgColor
    }
}
