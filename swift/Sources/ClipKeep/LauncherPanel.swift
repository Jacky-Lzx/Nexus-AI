import AppKit
import ApplicationServices
import QuartzCore

private enum Layout {
    static let width: CGFloat = 780
    static let collapsedHeight: CGFloat = 72
    static let expandedHeight: CGFloat = 520
    static let cornerRadius: CGFloat = 28
    static let outerInset: CGFloat = 8
    static let searchHeight: CGFloat = 56
    static let contentSpacing: CGFloat = 8
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

private final class ResultsSplitView: NSSplitView {
    var showsDivider = true {
        didSet {
            guard showsDivider != oldValue else { return }
            adjustSubviews()
            needsDisplay = true
        }
    }

    override var dividerThickness: CGFloat {
        showsDivider ? super.dividerThickness : 0
    }

    override func drawDivider(in rect: NSRect) {
        guard showsDivider else { return }
        super.drawDivider(in: rect)
    }

}

final class LauncherPanelController: NSWindowController,
    NSSearchFieldDelegate,
    NSSplitViewDelegate,
    NSTableViewDataSource,
    NSTableViewDelegate {

    private let coordinator: SearchCoordinator
    private let root = NSView()
    private let searchContainer = NSView()
    private let searchIcon = NSImageView()
    private let searchField = NSTextField()
    private let content = NSView()
    private let splitView = ResultsSplitView()
    private let listPane = NSView()
    private let detailPane = NSView()
    private let tableView = NSTableView()
    private let scrollView = NSScrollView()
    private let detailText = NSTextField(wrappingLabelWithString: "")
    private let detailImage = NSImageView()
    private let detailInfo = NSStackView()
    private let statusLabel = NSTextField(labelWithString: "ClipKeep")
    private var results: [SearchResult] = []
    private var expanded = false
    private var searchMode = SearchMode.applications
    private var hiding = false
    private var ignoreNextResign = false
    private var previousApplication: NSRunningApplication?
    private var listWidthConstraint: NSLayoutConstraint?
    private var scrollTrailingConstraint: NSLayoutConstraint?
    private var resetScrollOnNextResults = false

    var onPluginStateChanged: (() -> Void)?

    var imagePreviewQuality: ImagePreviewQuality {
        get {
            UserDefaults.standard.string(forKey: "clipboard.imagePreviewQuality")
                .flatMap(ImagePreviewQuality.init(rawValue:)) ?? .highDefinition
        }
        set {
            UserDefaults.standard.set(newValue.rawValue, forKey: "clipboard.imagePreviewQuality")
            updateDetails()
        }
    }

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
        if isPresented {
            if searchMode == mode {
                hide(restorePreviousApplication: true)
            } else {
                switchSearchMode(to: mode)
            }
        } else {
            show(mode: mode)
        }
    }

    func show(mode: SearchMode = .applications, query: String = "") {
        guard let panel = window as? LauncherPanel else { return }
        if let frontmost = NSWorkspace.shared.frontmostApplication,
           frontmost.processIdentifier != ProcessInfo.processInfo.processIdentifier {
            previousApplication = frontmost
        }
        searchMode = mode
        hiding = false
        searchField.stringValue = query
        searchField.placeholderString = mode.placeholder
        statusLabel.stringValue = mode.title
        updateModeLayout()
        requestSearch(query)
        setExpanded(shouldExpand(for: query), animated: false)
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

    func hide(restorePreviousApplication: Bool = false, pasteAfterRestore: Bool = false) {
        guard let panel = window, panel.isVisible, !hiding else { return }
        hiding = true
        let applicationToRestore = restorePreviousApplication ? previousApplication : nil
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.13
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self, weak panel] in
            panel?.orderOut(nil)
            panel?.alphaValue = 1
            self?.hiding = false
            applicationToRestore?.activate()
            if pasteAfterRestore, let applicationToRestore {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) {
                    Self.sendPaste(to: applicationToRestore)
                }
            }
        })
    }

    private func configurePanel(_ panel: NSPanel) {
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isMovableByWindowBackground = true
        panel.animationBehavior = .none
    }

    private func buildInterface() {
        guard let panel = window else { return }
        root.frame = panel.contentView?.bounds ?? .zero
        root.autoresizingMask = [.width, .height]
        root.wantsLayer = true
        root.layer?.cornerRadius = Layout.cornerRadius
        root.layer?.cornerCurve = .continuous
        root.layer?.masksToBounds = true
        installGlassSurface(in: panel)

        searchContainer.wantsLayer = true
        searchContainer.layer?.backgroundColor = NSColor.controlBackgroundColor
            .withAlphaComponent(0.22).cgColor
        searchContainer.layer?.cornerRadius = 18
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
            searchContainer.topAnchor.constraint(equalTo: root.topAnchor, constant: Layout.outerInset),
            searchContainer.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: Layout.outerInset),
            searchContainer.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -Layout.outerInset),
            searchContainer.heightAnchor.constraint(equalToConstant: Layout.searchHeight),
            searchIcon.leadingAnchor.constraint(equalTo: searchContainer.leadingAnchor, constant: 17),
            searchIcon.centerYAnchor.constraint(equalTo: searchContainer.centerYAnchor),
            searchIcon.widthAnchor.constraint(equalToConstant: 20),
            searchIcon.heightAnchor.constraint(equalToConstant: 20),
            searchField.leadingAnchor.constraint(equalTo: searchIcon.trailingAnchor, constant: 11),
            searchField.trailingAnchor.constraint(equalTo: searchContainer.trailingAnchor, constant: -12),
            searchField.centerYAnchor.constraint(equalTo: searchContainer.centerYAnchor),
            content.topAnchor.constraint(equalTo: searchContainer.bottomAnchor, constant: Layout.contentSpacing),
            content.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: Layout.outerInset),
            content.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -Layout.outerInset),
            content.heightAnchor.constraint(
                equalToConstant: Layout.expandedHeight
                    - Layout.outerInset * 2
                    - Layout.searchHeight
                    - Layout.contentSpacing
            ),
        ])
    }

    private func buildResults() {
        splitView.translatesAutoresizingMaskIntoConstraints = false
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.delegate = self
        content.addSubview(splitView)

        splitView.addArrangedSubview(listPane)
        splitView.addArrangedSubview(detailPane)
        listWidthConstraint = listPane.widthAnchor.constraint(equalToConstant: 316)
        listWidthConstraint?.isActive = true

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
        scrollView.hasVerticalScroller = false
        scrollView.autohidesScrollers = false
        scrollView.scrollerStyle = .legacy
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        listPane.addSubview(scrollView)

        detailText.font = .systemFont(ofSize: 15)
        detailText.textColor = .secondaryLabelColor
        detailText.maximumNumberOfLines = 8
        detailText.translatesAutoresizingMaskIntoConstraints = false
        detailImage.imageScaling = .scaleProportionallyUpOrDown
        detailImage.imageAlignment = .alignCenter
        detailImage.wantsLayer = true
        detailImage.layer?.cornerRadius = 12
        detailImage.layer?.cornerCurve = .continuous
        detailImage.layer?.masksToBounds = true
        detailImage.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.16).cgColor
        detailImage.translatesAutoresizingMaskIntoConstraints = false
        detailImage.isHidden = true
        detailInfo.orientation = .vertical
        detailInfo.spacing = 6
        detailInfo.alignment = .leading
        detailInfo.translatesAutoresizingMaskIntoConstraints = false
        detailPane.addSubview(detailText)
        detailPane.addSubview(detailImage)
        detailPane.addSubview(detailInfo)

        let footer = NSView()
        footer.translatesAutoresizingMaskIntoConstraints = false
        footer.wantsLayer = true
        footer.layer?.backgroundColor = NSColor.controlBackgroundColor
            .withAlphaComponent(0.14).cgColor
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

        scrollTrailingConstraint = scrollView.trailingAnchor.constraint(
            equalTo: listPane.trailingAnchor,
            constant: -8
        )
        NSLayoutConstraint.activate([
            splitView.topAnchor.constraint(equalTo: content.topAnchor),
            splitView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            splitView.bottomAnchor.constraint(equalTo: footer.topAnchor),
            scrollView.topAnchor.constraint(equalTo: listPane.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: listPane.leadingAnchor),
            scrollTrailingConstraint!,
            scrollView.bottomAnchor.constraint(equalTo: listPane.bottomAnchor),
            detailText.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 16),
            detailText.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 24),
            detailText.trailingAnchor.constraint(equalTo: detailPane.trailingAnchor, constant: -18),
            detailImage.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 16),
            detailImage.leadingAnchor.constraint(equalTo: detailText.leadingAnchor),
            detailImage.trailingAnchor.constraint(equalTo: detailText.trailingAnchor),
            detailImage.heightAnchor.constraint(equalToConstant: 250),
            detailInfo.leadingAnchor.constraint(equalTo: detailText.leadingAnchor),
            detailInfo.trailingAnchor.constraint(equalTo: detailText.trailingAnchor),
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
        updateModeLayout()
    }

    private func installGlassSurface(in panel: NSWindow) {
        let frame = panel.contentView?.bounds ?? .zero
        let glass = NSGlassEffectView(frame: frame)
        glass.autoresizingMask = [.width, .height]
        glass.contentView = root
        glass.cornerRadius = Layout.cornerRadius
        glass.style = .regular
        glass.tintColor = NSColor.windowBackgroundColor.withAlphaComponent(0.08)
        panel.contentView = glass
    }

    private func bindCoordinator() {
        coordinator.onResultsChanged = { [weak self] results in
            guard let self else { return }
            self.results = results
            self.tableView.reloadData()
            self.updateScrollerVisibility()
            if !results.isEmpty {
                self.tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            }
            if self.resetScrollOnNextResults {
                self.resetResultsScrollPosition()
                self.resetScrollOnNextResults = false
            }
            self.updateDetails()
        }
        coordinator.onStatusChanged = { [weak self] status in self?.statusLabel.stringValue = status }
        coordinator.onPluginStateChanged = { [weak self] in self?.onPluginStateChanged?() }
    }

    func controlTextDidChange(_ obj: Notification) {
        let query = searchField.stringValue
        requestSearch(query)
        setExpanded(shouldExpand(for: query), animated: true)
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
            hide(restorePreviousApplication: true); return true
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
        if result.kind == .command,
           result.metadata["route"] == "clipboard_history" {
            switchSearchMode(to: .clipboard)
        } else if result.kind.isClipboard {
            let canPaste = ensureAccessibilityPermission()
            coordinator.execute(result, actionId: "copy") { [weak self] success in
                guard let self, success else { return }
                self.hide(
                    restorePreviousApplication: true,
                    pasteAfterRestore: canPaste
                )
            }
        } else {
            coordinator.execute(result, actionId: "open")
            hide()
        }
    }

    private func switchSearchMode(to mode: SearchMode) {
        searchMode = mode
        searchField.stringValue = ""
        searchField.placeholderString = mode.placeholder
        statusLabel.stringValue = mode.title
        updateModeLayout()
        requestSearch("")
        setExpanded(shouldExpand(for: ""), animated: true)
        window?.makeFirstResponder(searchField)
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
        guard let result = selectedResult, result.kind.isClipboard else { return false }
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
        guard searchMode == .clipboard else { return }
        guard let result = selectedResult else {
            updateDetailInfo([])
            detailText.stringValue = results.isEmpty ? "No results" : "Select a result"
            detailImage.image = nil
            detailImage.isHidden = true
            detailText.isHidden = false
            return
        }
        detailText.stringValue = result.preview
        detailImage.image = previewImage(from: result)
        detailImage.isHidden = detailImage.image == nil
        detailText.isHidden = detailImage.image != nil
        var info = [
            ("Plugin", result.pluginId),
            ("Type", result.kind.rawValue.replacingOccurrences(of: "_", with: " ").capitalized),
        ]
        for (key, value) in result.metadata
            .filter({
                ![
                    "route",
                    "thumbnail_png",
                    "preview_image_path",
                    "original_image_path",
                ].contains($0.key)
            })
            .sorted(by: { $0.key < $1.key }) {
            info.append((key.capitalized, value))
        }
        updateDetailInfo(info)
    }

    private func updateModeLayout() {
        let showsDetails = searchMode == .clipboard
        splitView.showsDivider = showsDetails
        detailPane.isHidden = !showsDetails
        listWidthConstraint?.isActive = showsDetails
        scrollTrailingConstraint?.constant = showsDetails ? -8 : 0
        splitView.needsLayout = true
        updateScrollerVisibility()
    }

    private func updateScrollerVisibility() {
        scrollView.layoutSubtreeIfNeeded()
        tableView.layoutSubtreeIfNeeded()
        let contentHeight = results.indices.last
            .map { tableView.rect(ofRow: $0).maxY } ?? 0
        let needsScroller = contentHeight > scrollView.contentView.bounds.height + 0.5
        guard scrollView.hasVerticalScroller != needsScroller else { return }
        scrollView.hasVerticalScroller = needsScroller
        scrollView.verticalScroller?.controlSize = .small
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    private func requestSearch(_ query: String) {
        resetScrollOnNextResults = true
        resetResultsScrollPosition()
        coordinator.search(query, pluginId: searchMode.pluginId)
    }

    private func resetResultsScrollPosition() {
        scrollView.contentView.scroll(to: .zero)
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    func splitView(
        _ splitView: NSSplitView,
        effectiveRect proposedEffectiveRect: NSRect,
        forDrawnRect drawnRect: NSRect,
        ofDividerAt dividerIndex: Int
    ) -> NSRect {
        searchMode == .clipboard ? proposedEffectiveRect : .zero
    }

    func splitView(_ splitView: NSSplitView, shouldHideDividerAt dividerIndex: Int) -> Bool {
        searchMode == .applications
    }

    private func updateDetailInfo(_ items: [(String, String)]) {
        detailInfo.arrangedSubviews.forEach {
            detailInfo.removeArrangedSubview($0)
            $0.removeFromSuperview()
        }
        for index in stride(from: 0, to: items.count, by: 2) {
            let row = NSStackView()
            row.orientation = .horizontal
            row.distribution = .fillEqually
            row.spacing = 18
            row.addArrangedSubview(makeInfoItem(items[index]))
            if items.indices.contains(index + 1) {
                row.addArrangedSubview(makeInfoItem(items[index + 1]))
            } else {
                row.addArrangedSubview(NSView())
            }
            detailInfo.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: detailInfo.widthAnchor).isActive = true
        }
    }

    private func makeInfoItem(_ item: (String, String)) -> NSView {
        let (key, value) = item
        let container = NSStackView()
        container.orientation = .horizontal
        container.distribution = .fill
        container.spacing = 8
        let label = NSTextField(labelWithString: key)
        label.textColor = .secondaryLabelColor
        label.font = .systemFont(ofSize: 13)
        let field = NSTextField(labelWithString: value)
        field.textColor = .labelColor
        field.font = .systemFont(ofSize: 13, weight: .medium)
        field.lineBreakMode = .byTruncatingMiddle
        container.addArrangedSubview(label)
        container.addArrangedSubview(field)
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return container
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
            panel.animator().setFrame(frame, display: true)
            self.content.animator().alphaValue = shouldExpand ? 1 : 0
        }
        if animated {
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.24
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                changes()
            }, completionHandler: { [weak self] in
                self?.updateScrollerVisibility()
            })
        } else {
            content.alphaValue = shouldExpand ? 1 : 0
            panel.setFrame(frame, display: true)
            updateScrollerVisibility()
        }
    }

    private func shouldExpand(for query: String) -> Bool {
        searchMode == .clipboard
            || !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func ensureAccessibilityPermission() -> Bool {
        guard !AXIsProcessTrusted() else { return true }
        let promptKey = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
        AXIsProcessTrustedWithOptions([promptKey: true] as CFDictionary)
        statusLabel.stringValue = "Enable Accessibility to paste automatically"
        return false
    }

    private static func sendPaste(to application: NSRunningApplication) {
        guard AXIsProcessTrusted(),
              let source = CGEventSource(stateID: .hidSystemState),
              let keyDown = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: true),
              let keyUp = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: false)
        else { return }
        keyDown.flags = .maskCommand
        keyUp.flags = .maskCommand
        keyDown.postToPid(application.processIdentifier)
        keyUp.postToPid(application.processIdentifier)
    }

    private func previewImage(from result: SearchResult) -> NSImage? {
        let pathKey = imagePreviewQuality == .original
            ? "original_image_path"
            : "preview_image_path"
        if let path = result.metadata[pathKey],
           let image = NSImage(contentsOfFile: path) {
            return image
        }
        guard let encoded = result.metadata["thumbnail_png"],
              let data = Data(base64Encoded: encoded) else { return nil }
        return NSImage(data: data)
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
    private static let applicationIconCache = NSCache<NSString, NSImage>()

    private let symbol = NSTextField(labelWithString: "")
    private let artwork = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "")
    private let subtitleLabel = NSTextField(labelWithString: "")
    private var stackedTextConstraints: [NSLayoutConstraint] = []
    private var inlineTextConstraints: [NSLayoutConstraint] = []
    private var usesInlineSubtitle = false

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
        artwork.imageScaling = .scaleProportionallyUpOrDown
        artwork.wantsLayer = true
        artwork.layer?.cornerRadius = 8
        artwork.layer?.cornerCurve = .continuous
        artwork.layer?.masksToBounds = true
        artwork.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.font = .systemFont(ofSize: 15, weight: .semibold)
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        subtitleLabel.font = .systemFont(ofSize: 11)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingMiddle
        subtitleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(symbol)
        addSubview(artwork)
        addSubview(titleLabel)
        addSubview(subtitleLabel)
        NSLayoutConstraint.activate([
            symbol.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            symbol.centerYAnchor.constraint(equalTo: centerYAnchor),
            symbol.widthAnchor.constraint(equalToConstant: 36),
            symbol.heightAnchor.constraint(equalToConstant: 36),
            artwork.leadingAnchor.constraint(equalTo: symbol.leadingAnchor),
            artwork.centerYAnchor.constraint(equalTo: symbol.centerYAnchor),
            artwork.widthAnchor.constraint(equalTo: symbol.widthAnchor),
            artwork.heightAnchor.constraint(equalTo: symbol.heightAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: symbol.trailingAnchor, constant: 10),
            subtitleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
        stackedTextConstraints = [
            titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            subtitleLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            subtitleLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 3),
        ]
        inlineTextConstraints = [
            titleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            titleLabel.trailingAnchor.constraint(
                lessThanOrEqualTo: subtitleLabel.leadingAnchor,
                constant: -16
            ),
            subtitleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
        ]
        NSLayoutConstraint.activate(stackedTextConstraints)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ result: SearchResult, selected: Bool) {
        let image = Self.artworkImage(for: result)
        artwork.image = image
        artwork.isHidden = image == nil
        symbol.isHidden = image != nil
        switch result.kind {
        case .application: symbol.stringValue = "A"
        case .clipboardText: symbol.stringValue = "T"
        case .clipboardRichText: symbol.stringValue = "R"
        case .clipboardImage: symbol.stringValue = "I"
        case .clipboardFiles: symbol.stringValue = "F"
        case .command: symbol.stringValue = "⌘"
        default: symbol.stringValue = "·"
        }
        symbol.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.10).cgColor
        setInlineSubtitle(result.kind == .application)
        titleLabel.stringValue = result.title
        subtitleLabel.stringValue = result.subtitle
        layer?.backgroundColor = selected
            ? NSColor.white.withAlphaComponent(0.12).cgColor
            : NSColor.clear.cgColor
    }

    private func setInlineSubtitle(_ inline: Bool) {
        guard usesInlineSubtitle != inline else { return }
        usesInlineSubtitle = inline
        NSLayoutConstraint.deactivate(inline ? stackedTextConstraints : inlineTextConstraints)
        NSLayoutConstraint.activate(inline ? inlineTextConstraints : stackedTextConstraints)
    }

    private static func artworkImage(for result: SearchResult) -> NSImage? {
        if let encoded = result.metadata["thumbnail_png"],
           let data = Data(base64Encoded: encoded),
           let image = NSImage(data: data) {
            return image
        }
        guard result.kind == .application,
              let path = result.metadata["path"] else { return nil }
        let cacheKey = path as NSString
        if let image = applicationIconCache.object(forKey: cacheKey) {
            return image
        }
        let image = NSWorkspace.shared.icon(forFile: path)
        applicationIconCache.setObject(image, forKey: cacheKey)
        return image
    }
}
