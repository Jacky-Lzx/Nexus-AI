import AppKit
import ApplicationServices
import ImageIO
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
    var handleKeyEvent: ((NSEvent) -> Bool)?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func sendEvent(_ event: NSEvent) {
        if event.type == .keyDown, handleKeyEvent?(event) == true { return }
        super.sendEvent(event)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.modifierFlags.contains(.command) {
            if event.charactersIgnoringModifiers == "k", handleCommand?(NSSelectorFromString("showActions:")) == true {
                return true
            }
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

private final class ResultsTableView: NSTableView {
    var actionMenuProvider: (() -> NSMenu?)?

    override func menu(for event: NSEvent) -> NSMenu? {
        let clicked = row(at: convert(event.locationInWindow, from: nil))
        if clicked >= 0 {
            selectRowIndexes(IndexSet(integer: clicked), byExtendingSelection: false)
        }
        return actionMenuProvider?()
    }
}

private final class ReminderFormView: NSView {
    private let contentField = NSTextField()
    private let datePicker = NSDatePicker()

    var content: String { contentField.stringValue }
    var date: Date { datePicker.dateValue }

    override init(frame frameRect: NSRect) {
        super.init(frame: NSRect(x: 0, y: 0, width: 360, height: 92))

        let contentLabel = NSTextField(labelWithString: "Reminder")
        let timeLabel = NSTextField(labelWithString: "Time")
        for label in [contentLabel, timeLabel] {
            label.font = .systemFont(ofSize: 13, weight: .medium)
            label.translatesAutoresizingMaskIntoConstraints = false
            addSubview(label)
        }

        contentField.placeholderString = "What do you want to remember?"
        contentField.translatesAutoresizingMaskIntoConstraints = false
        addSubview(contentField)

        datePicker.datePickerStyle = .textFieldAndStepper
        datePicker.datePickerElements = [.yearMonthDay, .hourMinute]
        datePicker.minDate = Date(timeIntervalSinceNow: 60)
        datePicker.dateValue = Date(timeIntervalSinceNow: 3_600)
        datePicker.translatesAutoresizingMaskIntoConstraints = false
        addSubview(datePicker)

        NSLayoutConstraint.activate([
            contentLabel.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            contentLabel.leadingAnchor.constraint(equalTo: leadingAnchor),
            contentLabel.widthAnchor.constraint(equalToConstant: 72),
            contentField.centerYAnchor.constraint(equalTo: contentLabel.centerYAnchor),
            contentField.leadingAnchor.constraint(equalTo: contentLabel.trailingAnchor, constant: 8),
            contentField.trailingAnchor.constraint(equalTo: trailingAnchor),
            timeLabel.topAnchor.constraint(equalTo: contentLabel.bottomAnchor, constant: 20),
            timeLabel.leadingAnchor.constraint(equalTo: contentLabel.leadingAnchor),
            timeLabel.widthAnchor.constraint(equalTo: contentLabel.widthAnchor),
            datePicker.centerYAnchor.constraint(equalTo: timeLabel.centerYAnchor),
            datePicker.leadingAnchor.constraint(equalTo: contentField.leadingAnchor),
            datePicker.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
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

private final class PreviewImageView: NSImageView {
    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric)
    }
}

final class LauncherPanelController: NSWindowController,
    NSSearchFieldDelegate,
    NSSplitViewDelegate,
    NSTableViewDataSource,
    NSTableViewDelegate {

    private let coordinator: SearchCoordinator
    private let pluginNamesById: [String: String]
    private let root = NSView()
    private let searchContainer = NSView()
    private let searchIcon = NSImageView()
    private let searchField = NSTextField()
    private let content = NSView()
    private let splitView = ResultsSplitView()
    private let listPane = NSView()
    private let detailPane = NSView()
    private let tableView = ResultsTableView()
    private let scrollView = NSScrollView()
    private let detailText = NSTextField(wrappingLabelWithString: "")
    private let detailImage = PreviewImageView()
    private let detailInfo = NSStackView()
    private let statusLabel = NSTextField(labelWithString: "Nexus")
    private let externalTextEditor = ExternalTextEditor()
    private let menuBarProvider = MenuBarProvider()
    private var results: [SearchResult] = []
    private var expanded = false
    private var searchMode = SearchMode.global
    private var hiding = false
    private var ignoreNextResign = false
    private var previousApplication: NSRunningApplication?
    private var listWidthConstraint: NSLayoutConstraint?
    private var scrollTrailingConstraint: NSLayoutConstraint?
    private var resetScrollOnNextResults = false
    private var detailImageGeneration: UInt64 = 0

    private static let detailImageCache = NSCache<NSString, NSImage>()
    private static let thumbnailImageCache = NSCache<NSString, NSImage>()

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
        pluginNamesById = Dictionary(
            uniqueKeysWithValues: coordinator.descriptors.map { ($0.id, $0.name) }
        )
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
        panel.handleKeyEvent = { [weak self] event in self?.handleKeyEvent(event) ?? false }
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

    func show(mode: SearchMode = .global, query: String = "") {
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
        refreshMenuBarPlugin(for: mode)
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

    func hide(
        restorePreviousApplication: Bool = false,
        pasteAfterRestore: Bool = false,
        completion: (() -> Void)? = nil
    ) {
        guard let panel = window, panel.isVisible, !hiding else {
            completion?()
            return
        }
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
            completion?()
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
        searchField.placeholderString = SearchMode.global.placeholder
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
        tableView.actionMenuProvider = { [weak self] in self?.makeActionsMenu() }
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
        cell.configure(
            result,
            pluginName: pluginNamesById[result.pluginId] ?? result.pluginId,
            showsPluginName: showsPluginNameInCurrentMode,
            selected: row == tableView.selectedRow
        )
        return cell
    }

    private var showsPluginNameInCurrentMode: Bool {
        switch searchMode {
        case .clipboard:
            false
        case let .plugin(plugin):
            plugin.id != "io.clipkeep.kill-process"
        case .global:
            true
        }
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        tableView.reloadData(forRowIndexes: IndexSet(integersIn: 0..<results.count), columnIndexes: IndexSet(integer: 0))
        updateDetails()
    }

    @objc private func openSelected() {
        guard let result = selectedResult else { return }
        if result.kind == .command,
           let pluginId = result.metadata["route_plugin_id"],
           coordinator.isEnabled(pluginId),
           let plugin = coordinator.descriptors.first(where: { $0.id == pluginId }) {
            switchSearchMode(to: pluginId == "io.clipkeep.clipboard" ? .clipboard : .plugin(plugin))
        } else if let action = result.actions.first(where: { $0.default }) ?? result.actions.first {
            perform(action, on: result)
        }
    }

    private func perform(_ action: ResultAction, on result: SearchResult) {
        if result.pluginId == "io.clipkeep.clipboard", action.id == "edit" {
            performClipboardEdit(on: result)
            return
        }
        if result.pluginId == "io.clipkeep.reminders", action.id == "create" {
            performReminderCreation(on: result)
            return
        }
        if result.pluginId == "io.clipkeep.menu-bar", action.id == "press" {
            performMenuBarPress(on: result)
            return
        }
        if action.destructive {
            let alert = NSAlert()
            alert.messageText = result.metadata["confirmation_title"] ?? "\(action.title)?"
            alert.informativeText = result.metadata["confirmation_message"]
                ?? "This system action may interrupt your current work."
            alert.alertStyle = .warning
            alert.addButton(withTitle: action.title)
            alert.addButton(withTitle: "Cancel")
            ignoreNextResign = true
            let response = alert.runModal()
            window?.makeKey()
            window?.makeFirstResponder(searchField)
            guard response == .alertFirstButtonReturn else { return }
        }
        let canPaste = action.pasteToPrevious ? ensureAccessibilityPermission() : false
        coordinator.execute(result, actionId: action.id) { [weak self] success in
            guard let self, success, action.closesWindow else { return }
            self.hide(
                restorePreviousApplication: action.pasteToPrevious,
                pasteAfterRestore: action.pasteToPrevious && canPaste
            )
        }
        if action.closesWindow && !action.pasteToPrevious {
            hide()
        }
    }

    private func performReminderCreation(on result: SearchResult) {
        let form = ReminderFormView()
        let alert = NSAlert()
        alert.messageText = "Add Reminder"
        alert.informativeText = "Enter what you want to remember and when Nexus should notify you."
        alert.alertStyle = .informational
        alert.accessoryView = form
        alert.addButton(withTitle: "Add Reminder")
        alert.addButton(withTitle: "Cancel")
        ignoreNextResign = true
        guard alert.runModal() == .alertFirstButtonReturn else {
            window?.makeKey()
            window?.makeFirstResponder(searchField)
            return
        }
        let content = form.content.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty, form.date.timeIntervalSinceNow > 0 else {
            coordinator.onStatusChanged?("Enter reminder content and a future time")
            window?.makeKey()
            window?.makeFirstResponder(searchField)
            return
        }
        guard let data = try? JSONSerialization.data(withJSONObject: [
            "content": content,
            "due_at": Int(form.date.timeIntervalSince1970),
        ]), let input = String(data: data, encoding: .utf8) else { return }

        NotificationPresenter.shared.requestAuthorization(willPrompt: { [weak self] in
            self?.ignoreNextResign = true
        }) { [weak self] authorized in
            DispatchQueue.main.async {
                guard let self else { return }
                self.ignoreNextResign = false
                self.window?.makeKey()
                self.window?.makeFirstResponder(self.searchField)
                guard authorized else {
                    self.coordinator.onStatusChanged?("Allow notifications in System Settings to create reminders")
                    return
                }
                self.coordinator.execute(result, actionId: "create", input: input)
            }
        }
    }

    private func performMenuBarPress(on result: SearchResult) {
        guard ensureAccessibilityPermission(),
              let processIdentifier = result.metadata["target_pid"].flatMap(Int32.init),
              let menuItemID = result.metadata["menu_item_id"]
        else { return }
        let application = previousApplication?.processIdentifier == processIdentifier
            ? previousApplication
            : NSRunningApplication(processIdentifier: processIdentifier)
        guard let application else {
            coordinator.onStatusChanged?("The target application is no longer running")
            return
        }
        previousApplication = application
        hide(restorePreviousApplication: true) { [weak self] in
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) {
                guard let self else { return }
                if !self.menuBarProvider.pressItem(
                    processIdentifier: processIdentifier,
                    id: menuItemID
                ) {
                    self.coordinator.onStatusChanged?("Could not perform the selected menu command")
                }
            }
        }
    }

    private func performClipboardEdit(on result: SearchResult) {
        let applicationToRestore = previousApplication
        let canPaste = ensureAccessibilityPermission()
        hide { [weak self] in
            guard let self else { return }
            NSApp.deactivate()
            externalTextEditor.edit(result.preview) { [weak self] editResult in
                guard let self else {
                    applicationToRestore?.activate()
                    return
                }
                switch editResult {
                case let .failure(error):
                    coordinator.onStatusChanged?(error.localizedDescription)
                    applicationToRestore?.activate()
                case let .success(editedText):
                    coordinator.execute(result, actionId: "apply_edit", input: editedText) { success in
                        applicationToRestore?.activate()
                        guard success, canPaste, let applicationToRestore else { return }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) {
                            Self.sendPaste(to: applicationToRestore)
                        }
                    }
                }
            }
        }
    }

    private func makeActionsMenu() -> NSMenu? {
        guard let result = selectedResult, !result.actions.isEmpty else { return nil }
        let menu = NSMenu(title: "Actions")
        for action in result.actions {
            let item = NSMenuItem(
                title: action.title,
                action: #selector(performMenuAction(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = ActionInvocation(result: result, action: action)
            if action.destructive { item.attributedTitle = NSAttributedString(
                string: action.title,
                attributes: [.foregroundColor: NSColor.systemRed]
            ) }
            menu.addItem(item)
        }
        return menu
    }

    private func showActionsMenu() {
        guard let menu = makeActionsMenu(), tableView.selectedRow >= 0 else { return }
        let rowRect = tableView.rect(ofRow: tableView.selectedRow)
        ignoreNextResign = true
        menu.popUp(positioning: nil, at: NSPoint(x: rowRect.maxX - 8, y: rowRect.midY), in: tableView)
        window?.makeKey()
        window?.makeFirstResponder(searchField)
    }

    @objc private func performMenuAction(_ sender: NSMenuItem) {
        guard let invocation = sender.representedObject as? ActionInvocation else { return }
        perform(invocation.action, on: invocation.result)
    }

    private func switchSearchMode(to mode: SearchMode) {
        searchMode = mode
        searchField.stringValue = ""
        searchField.placeholderString = mode.placeholder
        statusLabel.stringValue = mode.title
        updateModeLayout()
        refreshMenuBarPlugin(for: mode)
        requestSearch("", preservingResults: true)
        setExpanded(shouldExpand(for: ""), animated: true)
        window?.makeFirstResponder(searchField)
    }

    private func refreshMenuBarPlugin(for mode: SearchMode) {
        let pluginId = "io.clipkeep.menu-bar"
        let needsMenuBar: Bool
        switch mode {
        case .global:
            needsMenuBar = coordinator.isEnabled(pluginId)
                && coordinator.isIncludedInGlobalSearch(pluginId)
        case let .plugin(plugin):
            needsMenuBar = plugin.id == pluginId && coordinator.isEnabled(pluginId)
        case .clipboard:
            needsMenuBar = false
        }
        guard needsMenuBar, ensureAccessibilityPermission(), let previousApplication else { return }
        let items = menuBarProvider.items(for: previousApplication)
        guard let data = try? JSONEncoder().encode(items),
              let encodedItems = String(data: data, encoding: .utf8)
        else { return }
        coordinator.configureRuntimeSettings([
            "target_pid": String(previousApplication.processIdentifier),
            "items": encodedItems,
        ], for: pluginId)
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

    private func handleKeyEvent(_ event: NSEvent) -> Bool {
        let relevantModifiers = event.modifierFlags.intersection([.command, .option, .control, .shift])
        if searchMode == .clipboard,
           relevantModifiers == [.command],
           event.charactersIgnoringModifiers == "\r",
           let result = selectedResult,
           let action = result.actions.first(where: { $0.id == "edit" }) {
            perform(action, on: result)
            return true
        }
        if LauncherKeyAction.moveDown.matches(event) {
            moveSelection(1)
            return true
        }
        if LauncherKeyAction.moveUp.matches(event) {
            moveSelection(-1)
            return true
        }
        return false
    }

    private func handle(_ selector: Selector) -> Bool {
        guard let result = selectedResult else { return false }
        if selector == NSSelectorFromString("showActions:") {
            showActionsMenu()
            return true
        }
        if selector == #selector(NSDocument.save(_:)) {
            guard let action = result.actions.first(where: { $0.id == "favorite" }) else { return false }
            perform(action, on: result); return true
        }
        if selector == #selector(NSResponder.deleteBackward(_:)) {
            guard let action = result.actions.first(where: { $0.id == "delete" }) else { return false }
            perform(action, on: result); return true
        }
        return false
    }

    private func updateDetails() {
        guard searchMode == .clipboard else { return }
        detailImageGeneration += 1
        let generation = detailImageGeneration
        guard let result = selectedResult else {
            updateDetailInfo([])
            detailText.stringValue = results.isEmpty ? "No results" : "Select a result"
            detailImage.image = nil
            detailImage.isHidden = true
            detailText.isHidden = false
            return
        }
        detailText.attributedStringValue = renderedClipboardPreview(result.preview)
        detailImage.image = thumbnailImage(from: result)
        detailImage.isHidden = detailImage.image == nil
        detailText.isHidden = detailImage.image != nil
        loadDetailImage(for: result, generation: generation)
        var info = [
            ("Plugin", result.pluginId),
            ("Type", result.kind.rawValue.replacingOccurrences(of: "_", with: " ").capitalized),
        ]
        for (key, value) in result.metadata
            .filter({
                ![
                    "route",
                    "thumbnail_png",
                    "thumbnail_image_path",
                    "preview_image_path",
                    "original_image_path",
                ].contains($0.key)
            })
            .sorted(by: { $0.key < $1.key }) {
            info.append((key.capitalized, value))
        }
        updateDetailInfo(info)
    }

    private func renderedClipboardPreview(_ text: String) -> NSAttributedString {
        let normalized = text
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 2
        let contentAttributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 15),
            .foregroundColor: NSColor.secondaryLabelColor,
            .paragraphStyle: paragraph,
        ]
        let newlineAttributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .semibold),
            .foregroundColor: NSColor.systemOrange.withAlphaComponent(0.82),
            .baselineOffset: -1,
            .paragraphStyle: paragraph,
        ]
        let output = NSMutableAttributedString()
        let lines = normalized.components(separatedBy: "\n")
        for (index, line) in lines.enumerated() {
            output.append(NSAttributedString(string: line, attributes: contentAttributes))
            if index < lines.count - 1 {
                output.append(NSAttributedString(string: "  ↵", attributes: newlineAttributes))
                output.append(NSAttributedString(string: "\n", attributes: contentAttributes))
            }
        }
        return output
    }

    private func updateModeLayout() {
        enforcePanelWidth()
        let showsDetails = searchMode.showsDetails
        splitView.showsDivider = showsDetails
        detailPane.isHidden = !showsDetails
        listWidthConstraint?.isActive = showsDetails
        scrollTrailingConstraint?.constant = showsDetails ? -8 : 0
        splitView.needsLayout = true
        updateScrollerVisibility()
    }

    private func enforcePanelWidth() {
        guard let panel = window, abs(panel.frame.width - Layout.width) > 0.5 else { return }
        var frame = panel.frame
        frame.origin.x += (frame.width - Layout.width) / 2
        frame.size.width = Layout.width
        panel.setFrame(frame, display: panel.isVisible)
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

    private func requestSearch(_ query: String, preservingResults: Bool = false) {
        resetScrollOnNextResults = true
        resetResultsScrollPosition()
        switch searchMode {
        case .global:
            coordinator.searchGlobal(query, preservingPublishedResults: preservingResults)
        case .clipboard:
            coordinator.search(
                query,
                pluginId: "io.clipkeep.clipboard",
                preservingPublishedResults: preservingResults
            )
        case let .plugin(plugin):
            coordinator.search(
                query,
                pluginId: plugin.id,
                preservingPublishedResults: preservingResults
            )
        }
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
        searchMode.showsDetails ? proposedEffectiveRect : .zero
    }

    func splitView(_ splitView: NSSplitView, shouldHideDividerAt dividerIndex: Int) -> Bool {
        !searchMode.showsDetails
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
        searchMode.expandsWhenEmpty
            || !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func ensureAccessibilityPermission() -> Bool {
        guard !AXIsProcessTrusted() else { return true }
        let promptKey = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
        AXIsProcessTrustedWithOptions([promptKey: true] as CFDictionary)
        statusLabel.stringValue = "Enable Accessibility to control other applications"
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

    private func thumbnailImage(from result: SearchResult) -> NSImage? {
        guard let path = result.metadata["thumbnail_image_path"] else { return nil }
        let key = path as NSString
        if let image = Self.thumbnailImageCache.object(forKey: key) { return image }
        guard let image = NSImage(contentsOfFile: path) else { return nil }
        Self.thumbnailImageCache.setObject(image, forKey: key)
        return image
    }

    private func loadDetailImage(for result: SearchResult, generation: UInt64) {
        guard result.kind == .clipboardImage,
              let originalPath = result.metadata["original_image_path"] else { return }
        let quality = imagePreviewQuality
        let preferredPath = quality == .highDefinition
            ? result.metadata["preview_image_path"] ?? originalPath
            : originalPath
        let sourcePath = FileManager.default.fileExists(atPath: preferredPath)
            ? preferredPath
            : originalPath
        let cacheKey = "\(quality.rawValue):\(sourcePath)" as NSString
        if let image = Self.detailImageCache.object(forKey: cacheKey) {
            detailImage.image = image
            detailImage.isHidden = false
            detailText.isHidden = true
            return
        }

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let image = Self.decodeImage(at: sourcePath, quality: quality) else { return }
            Self.detailImageCache.setObject(image, forKey: cacheKey)
            DispatchQueue.main.async {
                guard let self,
                      self.detailImageGeneration == generation,
                      self.selectedResult?.id == result.id,
                      self.imagePreviewQuality == quality
                else { return }
                self.detailImage.image = image
                self.detailImage.isHidden = false
                self.detailText.isHidden = true
            }
        }
    }

    private static func decodeImage(at path: String, quality: ImagePreviewQuality) -> NSImage? {
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil)
        else { return nil }
        let cgImage: CGImage?
        switch quality {
        case .highDefinition:
            let options: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: 1280,
                kCGImageSourceShouldCacheImmediately: true,
            ]
            cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
        case .original:
            cgImage = CGImageSourceCreateImageAtIndex(
                source,
                0,
                [kCGImageSourceShouldCacheImmediately: true] as CFDictionary
            )
        }
        guard let cgImage else { return nil }
        return NSImage(cgImage: cgImage, size: .zero)
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

private final class ActionInvocation: NSObject {
    let result: SearchResult
    let action: ResultAction

    init(result: SearchResult, action: ResultAction) {
        self.result = result
        self.action = action
    }
}

private final class ResultCellView: NSTableCellView {
    private static let applicationIconCache = NSCache<NSString, NSImage>()

    private let symbol = NSTextField(labelWithString: "")
    private let artwork = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "")
    private let multilineIndicator = NSTextField(labelWithString: "↩")
    private let subtitleLabel = NSTextField(labelWithString: "")
    private let pluginLabel = NSTextField(labelWithString: "")
    private var stackedTextConstraints: [NSLayoutConstraint] = []
    private var inlineTextConstraints: [NSLayoutConstraint] = []
    private var multilineIndicatorSpacingConstraint: NSLayoutConstraint?
    private var artworkWidthConstraint: NSLayoutConstraint?
    private var artworkHeightConstraint: NSLayoutConstraint?
    private var usesInlineSubtitle = false

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.cornerRadius = 12
        layer?.cornerCurve = .continuous
        layer?.masksToBounds = true
        symbol.font = .systemFont(ofSize: 18, weight: .semibold)
        symbol.alignment = .center
        symbol.wantsLayer = true
        symbol.layer?.cornerRadius = 8
        symbol.translatesAutoresizingMaskIntoConstraints = false
        artwork.imageScaling = .scaleProportionallyUpOrDown
        artwork.contentTintColor = .labelColor
        artwork.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 20, weight: .regular)
        artwork.wantsLayer = true
        artwork.layer?.cornerRadius = 8
        artwork.layer?.cornerCurve = .continuous
        artwork.layer?.masksToBounds = true
        artwork.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.font = .systemFont(ofSize: 15, weight: .semibold)
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.maximumNumberOfLines = 1
        titleLabel.usesSingleLineMode = true
        titleLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        multilineIndicator.font = .systemFont(ofSize: 15, weight: .semibold)
        multilineIndicator.textColor = .labelColor
        multilineIndicator.isHidden = true
        multilineIndicator.setContentHuggingPriority(.required, for: .horizontal)
        multilineIndicator.setContentCompressionResistancePriority(.required, for: .horizontal)
        multilineIndicator.translatesAutoresizingMaskIntoConstraints = false
        subtitleLabel.font = .systemFont(ofSize: 11)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingMiddle
        subtitleLabel.maximumNumberOfLines = 1
        subtitleLabel.usesSingleLineMode = true
        subtitleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false
        pluginLabel.font = .systemFont(ofSize: 11, weight: .medium)
        pluginLabel.textColor = .tertiaryLabelColor
        pluginLabel.alignment = .right
        pluginLabel.lineBreakMode = .byTruncatingTail
        pluginLabel.maximumNumberOfLines = 1
        pluginLabel.usesSingleLineMode = true
        pluginLabel.setContentHuggingPriority(.required, for: .horizontal)
        pluginLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        pluginLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(symbol)
        addSubview(artwork)
        addSubview(titleLabel)
        addSubview(multilineIndicator)
        addSubview(subtitleLabel)
        addSubview(pluginLabel)
        multilineIndicatorSpacingConstraint = multilineIndicator.leadingAnchor.constraint(
            equalTo: titleLabel.trailingAnchor,
            constant: 0
        )
        artworkWidthConstraint = artwork.widthAnchor.constraint(equalToConstant: 36)
        artworkHeightConstraint = artwork.heightAnchor.constraint(equalToConstant: 36)
        NSLayoutConstraint.activate([
            symbol.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            symbol.centerYAnchor.constraint(equalTo: centerYAnchor),
            symbol.widthAnchor.constraint(equalToConstant: 36),
            symbol.heightAnchor.constraint(equalToConstant: 36),
            artwork.centerXAnchor.constraint(equalTo: symbol.centerXAnchor),
            artwork.centerYAnchor.constraint(equalTo: symbol.centerYAnchor),
            artworkWidthConstraint!,
            artworkHeightConstraint!,
            titleLabel.leadingAnchor.constraint(equalTo: symbol.trailingAnchor, constant: 10),
            multilineIndicatorSpacingConstraint!,
            multilineIndicator.trailingAnchor.constraint(lessThanOrEqualTo: pluginLabel.leadingAnchor, constant: -12),
            multilineIndicator.firstBaselineAnchor.constraint(
                equalTo: titleLabel.firstBaselineAnchor,
                constant: 3
            ),
            pluginLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            pluginLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            pluginLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 150),
        ])
        stackedTextConstraints = [
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: pluginLabel.leadingAnchor, constant: -12),
            subtitleLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            subtitleLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 3),
            subtitleLabel.trailingAnchor.constraint(lessThanOrEqualTo: pluginLabel.leadingAnchor, constant: -12),
        ]
        inlineTextConstraints = [
            titleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            titleLabel.trailingAnchor.constraint(
                lessThanOrEqualTo: subtitleLabel.leadingAnchor,
                constant: -16
            ),
            subtitleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            subtitleLabel.trailingAnchor.constraint(
                lessThanOrEqualTo: pluginLabel.leadingAnchor,
                constant: -12
            ),
        ]
        NSLayoutConstraint.activate(stackedTextConstraints)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(
        _ result: SearchResult,
        pluginName: String,
        showsPluginName: Bool,
        selected: Bool
    ) {
        let image = Self.artworkImage(for: result)
        artwork.image = image
        artwork.isHidden = image == nil
        symbol.isHidden = image != nil
        let artworkSize: CGFloat = Self.usesFlatClipboardIcon(result) ? 18 : 36
        artworkWidthConstraint?.constant = artworkSize
        artworkHeightConstraint?.constant = artworkSize
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
        let hasMultipleLines = result.title.hasSuffix(" ↩")
        titleLabel.stringValue = hasMultipleLines ? String(result.title.dropLast(2)) : result.title
        multilineIndicator.stringValue = hasMultipleLines ? "↩" : ""
        multilineIndicator.isHidden = !hasMultipleLines
        multilineIndicatorSpacingConstraint?.constant = hasMultipleLines ? 4 : 0
        subtitleLabel.stringValue = result.subtitle
        pluginLabel.stringValue = showsPluginName ? pluginName : ""
        pluginLabel.isHidden = !showsPluginName
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
        if let symbolName = result.metadata["symbol_name"] {
            return NSImage(systemSymbolName: symbolName, accessibilityDescription: result.title)
        }
        if let path = result.metadata["thumbnail_image_path"] {
            let cacheKey = path as NSString
            if let image = applicationIconCache.object(forKey: cacheKey) { return image }
            guard let image = NSImage(contentsOfFile: path) else { return nil }
            applicationIconCache.setObject(image, forKey: cacheKey)
            return image
        }
        let clipboardSymbol: String? = switch result.kind {
        case .clipboardText: "doc.plaintext"
        case .clipboardRichText: "doc.richtext"
        case .clipboardImage: "photo"
        case .clipboardFiles: "doc.on.doc"
        default: nil
        }
        if let clipboardSymbol {
            return NSImage(
                systemSymbolName: clipboardSymbol,
                accessibilityDescription: result.kind.rawValue
            )
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

    private static func usesFlatClipboardIcon(_ result: SearchResult) -> Bool {
        switch result.kind {
        case .clipboardText, .clipboardRichText, .clipboardFiles:
            true
        case .clipboardImage:
            result.metadata["thumbnail_image_path"] == nil
        default:
            false
        }
    }
}
