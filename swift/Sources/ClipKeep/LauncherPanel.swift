import AppKit
import QuartzCore

private enum Layout {
    static let width: CGFloat = 780
    static let collapsedHeight: CGFloat = 84
    static let expandedHeight: CGFloat = 520
    static let cornerRadius: CGFloat = 22
}

final class LauncherPanel: NSPanel {
    var handleCommand: ((Selector) -> Bool)?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.modifierFlags.contains(.command) {
            if event.charactersIgnoringModifiers == "s", handleCommand?(#selector(NSDocument.save(_:))) == true {
                return true
            }
            if event.keyCode == 51, handleCommand?(#selector(NSResponder.deleteBackward(_:))) == true {
                return true
            }
        }
        return super.performKeyEquivalent(with: event)
    }
}

final class LauncherPanelController: NSWindowController,
    NSSearchFieldDelegate,
    NSTableViewDataSource,
    NSTableViewDelegate {

    private let coordinator: SearchCoordinator
    private let root = NSVisualEffectView()
    private let searchContainer = NSView()
    private let searchIcon = NSImageView()
    private let searchField = NSTextField()
    private let content = NSView()
    private let tableView = NSTableView()
    private let scrollView = NSScrollView()
    private let detailTitle = NSTextField(labelWithString: "")
    private let detailText = NSTextField(wrappingLabelWithString: "")
    private let detailInfo = NSStackView()
    private let statusLabel = NSTextField(labelWithString: "ClipKeep")
    private var results: [SearchResult] = []
    private var expanded = false
    private var searchMode = SearchMode.applications
    private var hiding = false
    private var ignoreNextResign = false

    var onPluginStateChanged: (() -> Void)?

    init(coordinator: SearchCoordinator) {
        self.coordinator = coordinator
        let panel = LauncherPanel(
            contentRect: NSRect(x: 0, y: 0, width: Layout.width, height: Layout.collapsedHeight),
            styleMask: [.borderless, .nonactivatingPanel, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        super.init(window: panel)
        configurePanel(panel)
        buildInterface()
        bindCoordinator()
        panel.handleCommand = { [weak self] selector in self?.handle(selector) ?? false }
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(windowResignedKey),
            name: NSWindow.didResignKeyNotification,
            object: panel
        )
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var isPresented: Bool { window?.isVisible == true && !hiding }

    func toggle(mode: SearchMode) {
        if isPresented, searchMode == mode {
            hide()
        } else {
            show(mode: mode)
        }
    }

    func show(mode: SearchMode = .applications, query: String = "") {
        guard let panel = window as? LauncherPanel else { return }
        searchMode = mode
        hiding = false
        searchField.stringValue = query
        searchField.placeholderString = mode.placeholder
        statusLabel.stringValue = mode.title
        coordinator.search(query, pluginId: mode.pluginId)
        setExpanded(!query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, animated: false)
        position(panel)
        panel.alphaValue = 0
        panel.orderFrontRegardless()
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKey()
        panel.makeFirstResponder(searchField)
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.18
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
        }
    }

    func hide() {
        guard let panel = window, panel.isVisible, !hiding else { return }
        hiding = true
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.13
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self, weak panel] in
            panel?.orderOut(nil)
            panel?.alphaValue = 1
            self?.hiding = false
        })
    }

    private func configurePanel(_ panel: NSPanel) {
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.isMovableByWindowBackground = true
        panel.animationBehavior = .none
    }

    private func buildInterface() {
        guard let panel = window else { return }
        root.frame = panel.contentView?.bounds ?? .zero
        root.autoresizingMask = [.width, .height]
        root.material = .hudWindow
        root.blendingMode = .behindWindow
        root.state = .active
        root.wantsLayer = true
        root.layer?.cornerRadius = Layout.cornerRadius
        root.layer?.cornerCurve = .continuous
        root.layer?.masksToBounds = true
        root.layer?.borderWidth = 1
        root.layer?.borderColor = NSColor.white.withAlphaComponent(0.16).cgColor
        panel.contentView = root

        searchContainer.wantsLayer = true
        searchContainer.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.07).cgColor
        searchContainer.layer?.cornerRadius = 15
        searchContainer.layer?.cornerCurve = .continuous
        searchContainer.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(searchContainer)

        searchField.translatesAutoresizingMaskIntoConstraints = false
        searchField.delegate = self
        searchIcon.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        searchIcon.contentTintColor = .secondaryLabelColor
        searchIcon.translatesAutoresizingMaskIntoConstraints = false
        searchContainer.addSubview(searchIcon)
        searchField.placeholderString = SearchMode.applications.placeholder
        searchField.font = .systemFont(ofSize: 24, weight: .regular)
        searchField.textColor = .labelColor
        searchField.focusRingType = .none
        searchField.isBezeled = false
        searchField.drawsBackground = false
        searchField.usesSingleLineMode = true
        searchContainer.addSubview(searchField)

        content.translatesAutoresizingMaskIntoConstraints = false
        content.alphaValue = 0
        root.addSubview(content)
        buildResults()

        NSLayoutConstraint.activate([
            searchContainer.topAnchor.constraint(equalTo: root.topAnchor, constant: 14),
            searchContainer.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 14),
            searchContainer.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -14),
            searchContainer.heightAnchor.constraint(equalToConstant: 56),
            searchIcon.leadingAnchor.constraint(equalTo: searchContainer.leadingAnchor, constant: 17),
            searchIcon.centerYAnchor.constraint(equalTo: searchContainer.centerYAnchor),
            searchIcon.widthAnchor.constraint(equalToConstant: 20),
            searchIcon.heightAnchor.constraint(equalToConstant: 20),
            searchField.leadingAnchor.constraint(equalTo: searchIcon.trailingAnchor, constant: 11),
            searchField.trailingAnchor.constraint(equalTo: searchContainer.trailingAnchor, constant: -12),
            searchField.centerYAnchor.constraint(equalTo: searchContainer.centerYAnchor),
            searchField.heightAnchor.constraint(equalToConstant: 40),
            content.topAnchor.constraint(equalTo: searchContainer.bottomAnchor, constant: 12),
            content.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 14),
            content.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -14),
            content.heightAnchor.constraint(equalToConstant: Layout.expandedHeight - 94),
        ])
    }

    private func buildResults() {
        let split = NSSplitView()
        split.translatesAutoresizingMaskIntoConstraints = false
        split.isVertical = true
        split.dividerStyle = .thin
        content.addSubview(split)

        let listPane = NSView()
        let detailPane = NSView()
        split.addArrangedSubview(listPane)
        split.addArrangedSubview(detailPane)
        listPane.widthAnchor.constraint(equalToConstant: 316).isActive = true

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("result"))
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 58
        tableView.intercellSpacing = NSSize(width: 0, height: 4)
        tableView.backgroundColor = .clear
        tableView.selectionHighlightStyle = .none
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(openSelected)
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        listPane.addSubview(scrollView)

        detailTitle.font = .systemFont(ofSize: 19, weight: .semibold)
        detailTitle.textColor = .labelColor
        detailTitle.lineBreakMode = .byTruncatingTail
        detailTitle.translatesAutoresizingMaskIntoConstraints = false
        detailText.font = .systemFont(ofSize: 15)
        detailText.textColor = .secondaryLabelColor
        detailText.maximumNumberOfLines = 8
        detailText.translatesAutoresizingMaskIntoConstraints = false
        detailInfo.orientation = .vertical
        detailInfo.spacing = 6
        detailInfo.alignment = .leading
        detailInfo.translatesAutoresizingMaskIntoConstraints = false
        detailPane.addSubview(detailTitle)
        detailPane.addSubview(detailText)
        detailPane.addSubview(detailInfo)

        let footer = NSView()
        footer.translatesAutoresizingMaskIntoConstraints = false
        footer.wantsLayer = true
        footer.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.13).cgColor
        content.addSubview(footer)
        statusLabel.font = .systemFont(ofSize: 13, weight: .medium)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        footer.addSubview(statusLabel)
        let hint = NSTextField(labelWithString: "↵ Run     ↑↓ Navigate     ⌘⇧Space Apps     ⌘⇧C Clipboard")
        hint.font = .systemFont(ofSize: 12, weight: .medium)
        hint.textColor = .tertiaryLabelColor
        hint.translatesAutoresizingMaskIntoConstraints = false
        footer.addSubview(hint)

        NSLayoutConstraint.activate([
            split.topAnchor.constraint(equalTo: content.topAnchor),
            split.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            split.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            split.bottomAnchor.constraint(equalTo: footer.topAnchor),
            scrollView.topAnchor.constraint(equalTo: listPane.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: listPane.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: listPane.trailingAnchor, constant: -8),
            scrollView.bottomAnchor.constraint(equalTo: listPane.bottomAnchor),
            detailTitle.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 12),
            detailTitle.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 24),
            detailTitle.trailingAnchor.constraint(equalTo: detailPane.trailingAnchor, constant: -18),
            detailText.topAnchor.constraint(equalTo: detailTitle.bottomAnchor, constant: 16),
            detailText.leadingAnchor.constraint(equalTo: detailTitle.leadingAnchor),
            detailText.trailingAnchor.constraint(equalTo: detailTitle.trailingAnchor),
            detailInfo.leadingAnchor.constraint(equalTo: detailTitle.leadingAnchor),
            detailInfo.trailingAnchor.constraint(equalTo: detailTitle.trailingAnchor),
            detailInfo.bottomAnchor.constraint(equalTo: detailPane.bottomAnchor, constant: -18),
            footer.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            footer.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            footer.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            footer.heightAnchor.constraint(equalToConstant: 46),
            statusLabel.leadingAnchor.constraint(equalTo: footer.leadingAnchor, constant: 12),
            statusLabel.centerYAnchor.constraint(equalTo: footer.centerYAnchor),
            hint.trailingAnchor.constraint(equalTo: footer.trailingAnchor, constant: -12),
            hint.centerYAnchor.constraint(equalTo: footer.centerYAnchor),
        ])
    }

    private func bindCoordinator() {
        coordinator.onResultsChanged = { [weak self] results in
            guard let self else { return }
            self.results = results
            self.tableView.reloadData()
            if !results.isEmpty {
                self.tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            }
            self.updateDetails()
        }
        coordinator.onStatusChanged = { [weak self] status in self?.statusLabel.stringValue = status }
        coordinator.onPluginStateChanged = { [weak self] in self?.onPluginStateChanged?() }
    }

    func controlTextDidChange(_ obj: Notification) {
        let query = searchField.stringValue
        coordinator.search(query, pluginId: searchMode.pluginId)
        setExpanded(!query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, animated: true)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.moveDown(_:)):
            moveSelection(1); return true
        case #selector(NSResponder.moveUp(_:)):
            moveSelection(-1); return true
        case #selector(NSResponder.insertNewline(_:)):
            openSelected(); return true
        case #selector(NSResponder.cancelOperation(_:)):
            hide(); return true
        default:
            return false
        }
    }

    func numberOfRows(in tableView: NSTableView) -> Int { results.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let result = results[row]
        let id = NSUserInterfaceItemIdentifier("ResultCell")
        let cell = tableView.makeView(withIdentifier: id, owner: self) as? ResultCellView ?? ResultCellView()
        cell.identifier = id
        cell.configure(result, selected: row == tableView.selectedRow)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        tableView.reloadData(forRowIndexes: IndexSet(integersIn: 0..<results.count), columnIndexes: IndexSet(integer: 0))
        updateDetails()
    }

    @objc private func openSelected() {
        guard let result = selectedResult else { return }
        coordinator.execute(result, actionId: result.kind == .application ? "open" : "copy")
        hide()
    }

    private var selectedResult: SearchResult? {
        guard results.indices.contains(tableView.selectedRow) else { return nil }
        return results[tableView.selectedRow]
    }

    private func moveSelection(_ delta: Int) {
        guard !results.isEmpty else { return }
        let current = max(tableView.selectedRow, 0)
        let next = min(max(current + delta, 0), results.count - 1)
        tableView.selectRowIndexes(IndexSet(integer: next), byExtendingSelection: false)
        tableView.scrollRowToVisible(next)
    }

    private func handle(_ selector: Selector) -> Bool {
        guard let result = selectedResult, result.kind == .clipboardText else { return false }
        if selector == #selector(NSDocument.save(_:)) {
            coordinator.execute(result, actionId: "favorite")
            return true
        }
        if selector == #selector(NSResponder.deleteBackward(_:)) {
            coordinator.execute(result, actionId: "delete")
            return true
        }
        return false
    }

    private func updateDetails() {
        detailInfo.arrangedSubviews.forEach { detailInfo.removeArrangedSubview($0); $0.removeFromSuperview() }
        guard let result = selectedResult else {
            detailTitle.stringValue = results.isEmpty ? "No results" : "Select a result"
            detailText.stringValue = ""
            return
        }
        detailTitle.stringValue = result.title
        detailText.stringValue = result.preview
        addInfo("Plugin", result.pluginId)
        addInfo("Type", result.kind.rawValue.replacingOccurrences(of: "_", with: " ").capitalized)
        for (key, value) in result.metadata.sorted(by: { $0.key < $1.key }) {
            addInfo(key.capitalized, value)
        }
    }

    private func addInfo(_ key: String, _ value: String) {
        let row = NSStackView()
        row.orientation = .horizontal
        row.distribution = .fill
        let label = NSTextField(labelWithString: key)
        label.textColor = .secondaryLabelColor
        label.font = .systemFont(ofSize: 13)
        let field = NSTextField(labelWithString: value)
        field.textColor = .labelColor
        field.font = .systemFont(ofSize: 13, weight: .medium)
        field.lineBreakMode = .byTruncatingMiddle
        row.addArrangedSubview(label)
        row.addArrangedSubview(field)
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        detailInfo.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: detailInfo.widthAnchor).isActive = true
    }

    private func setExpanded(_ shouldExpand: Bool, animated: Bool) {
        guard expanded != shouldExpand || !animated else { return }
        expanded = shouldExpand
        guard let panel = window else { return }
        var frame = panel.frame
        let newHeight = shouldExpand ? Layout.expandedHeight : Layout.collapsedHeight
        frame.origin.y += frame.height - newHeight
        frame.size.height = newHeight
        let changes = {
            panel.setFrame(frame, display: true)
            self.content.animator().alphaValue = shouldExpand ? 1 : 0
        }
        if animated {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.24
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                changes()
            }
        } else {
            content.alphaValue = shouldExpand ? 1 : 0
            panel.setFrame(frame, display: true)
        }
    }

    private func position(_ panel: NSPanel) {
        let screen = NSScreen.main ?? NSScreen.screens.first
        guard let visible = screen?.visibleFrame else { return }
        let x = visible.midX - Layout.width / 2
        let top = visible.midY + Layout.expandedHeight / 2
        panel.setFrameOrigin(NSPoint(x: x, y: top - panel.frame.height))
    }

    @objc private func windowResignedKey() {
        guard !ignoreNextResign else { ignoreNextResign = false; return }
        hide()
    }
}

private final class ResultCellView: NSTableCellView {
    private let symbol = NSTextField(labelWithString: "")
    private let titleLabel = NSTextField(labelWithString: "")
    private let subtitleLabel = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.cornerRadius = 12
        layer?.cornerCurve = .continuous
        symbol.font = .systemFont(ofSize: 18, weight: .semibold)
        symbol.alignment = .center
        symbol.wantsLayer = true
        symbol.layer?.cornerRadius = 8
        symbol.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.font = .systemFont(ofSize: 15, weight: .semibold)
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        subtitleLabel.font = .systemFont(ofSize: 11)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingMiddle
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(symbol)
        addSubview(titleLabel)
        addSubview(subtitleLabel)
        NSLayoutConstraint.activate([
            symbol.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            symbol.centerYAnchor.constraint(equalTo: centerYAnchor),
            symbol.widthAnchor.constraint(equalToConstant: 36),
            symbol.heightAnchor.constraint(equalToConstant: 36),
            titleLabel.leadingAnchor.constraint(equalTo: symbol.trailingAnchor, constant: 10),
            titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            subtitleLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            subtitleLabel.trailingAnchor.constraint(equalTo: titleLabel.trailingAnchor),
            subtitleLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 3),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ result: SearchResult, selected: Bool) {
        symbol.stringValue = result.kind == .application ? "A" : "T"
        symbol.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.10).cgColor
        titleLabel.stringValue = result.title
        subtitleLabel.stringValue = result.subtitle
        layer?.backgroundColor = selected
            ? NSColor.controlAccentColor.withAlphaComponent(0.28).cgColor
            : NSColor.clear.cgColor
    }
}
