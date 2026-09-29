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
            if event.charactersIgnoringModifiers == ",", handleCommand?(NSSelectorFromString("showSettings:")) == true {
                return true
            }
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

private struct ActiveActionInput {
    let result: SearchResult
    let action: ResultAction
    let fields: [(ActionInputField, NSTextField)]
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
    private var pluginNamesById: [String: String]
    private let root = NSView()
    private let searchContainer = NSView()
    private let searchIcon = NSImageView()
    private let searchField = NSTextField()
    private let actionInputStack = NSStackView()
    private let presentationContainer = NSView()
    private let presentationStack = NSStackView()
    private let content = NSView()
    private let splitView = ResultsSplitView()
    private let listPane = NSView()
    private let detailPane = NSView()
    private let tableView = ResultsTableView()
    private let scrollView = NSScrollView()
    private let textPreviewScroll = NSScrollView()
    private let textPreview = NSTextView()
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
    private var presentationTopConstraint: NSLayoutConstraint?
    private var presentationHeightConstraint: NSLayoutConstraint?
    private var contentTopConstraint: NSLayoutConstraint?
    private var footerHeightConstraint: NSLayoutConstraint?
    private var presentationValueLabels: [(NSTextField, PluginPresentationItem)] = []
    private var presentationTimer: Timer?
    private var activeActionInput: ActiveActionInput?
    private var inlineActionInput = false
    private var searchTrailingConstraint: NSLayoutConstraint!
    private var inputLeadingConstraint: NSLayoutConstraint!
    private var inlineInputConstraints: [NSLayoutConstraint] = []
    private var inlineQueryWidthConstraint: NSLayoutConstraint!
    private var resetScrollOnNextResults = false
    private var detailImageGeneration: UInt64 = 0
    private var expansionAnimationGeneration: UInt64 = 0

    private static let detailImageCache = NSCache<NSString, NSImage>()
    private static let thumbnailImageCache = NSCache<NSString, NSImage>()

    var onPluginStateChanged: (() -> Void)?
    var onShowSettings: (() -> Void)?

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
        var pluginNames = Dictionary(
            uniqueKeysWithValues: coordinator.descriptors.map { ($0.id, $0.name) }
        )
        pluginNames["io.clipkeep.plugins"] = "Nexus"
        pluginNames["io.clipkeep.settings"] = "Nexus"
        pluginNamesById = pluginNames
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
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(refreshSearchCaret),
            name: NSWindow.didBecomeKeyNotification,
            object: panel
        )
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var isPresented: Bool { window?.isVisible == true && !hiding }

    func refreshPlugins() {
        pluginNamesById = Dictionary(uniqueKeysWithValues: coordinator.descriptors.map { ($0.id, $0.name) })
        pluginNamesById["io.clipkeep.plugins"] = "Nexus"
        pluginNamesById["io.clipkeep.settings"] = "Nexus"
        if let input = activeActionInput,
           !coordinator.canExecute(input.result) {
            endActionInputForm(restoreSearchFocus: true)
        }
        switch searchMode {
        case let .plugin(plugin):
            guard let updated = coordinator.descriptors.first(where: { $0.id == plugin.id }),
                  coordinator.isEnabled(plugin.id) else {
                switchSearchMode(to: .global)
                return
            }
            searchMode = .plugin(updated)
            searchField.placeholderString = searchMode.placeholder
            statusLabel.stringValue = searchMode.title
        case .clipboard where !coordinator.isEnabled("io.clipkeep.clipboard"):
            switchSearchMode(to: .global)
        default: break
        }
        if isPresented { refreshMenuBarPlugin(for: searchMode) }
        tableView.reloadData()
    }

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
        endActionInputForm(restoreSearchFocus: false)
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
        updatePresentationItems()
        refreshMenuBarPlugin(for: mode)
        requestSearch(query)
        setExpanded(shouldExpand(for: query), animated: false)
        position(panel)
        panel.alphaValue = 0
        panel.orderFrontRegardless()
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKey()
        panel.makeFirstResponder(searchField)
        refreshSearchCaret()
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
        coordinator.cancelPendingActivation()
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

        actionInputStack.orientation = .horizontal
        actionInputStack.alignment = .centerY
        actionInputStack.spacing = 10
        actionInputStack.isHidden = true
        actionInputStack.translatesAutoresizingMaskIntoConstraints = false
        searchContainer.addSubview(actionInputStack)

        presentationContainer.translatesAutoresizingMaskIntoConstraints = false
        presentationContainer.isHidden = true
        root.addSubview(presentationContainer)
        presentationStack.orientation = .vertical
        presentationStack.spacing = 6
        presentationStack.translatesAutoresizingMaskIntoConstraints = false
        presentationContainer.addSubview(presentationStack)

        content.translatesAutoresizingMaskIntoConstraints = false
        content.alphaValue = 0
        content.isHidden = true
        root.addSubview(content)
        buildResults()

        presentationTopConstraint = presentationContainer.topAnchor.constraint(
            equalTo: searchContainer.bottomAnchor
        )
        presentationHeightConstraint = presentationContainer.heightAnchor.constraint(equalToConstant: 0)
        contentTopConstraint = content.topAnchor.constraint(equalTo: presentationContainer.bottomAnchor)
        searchTrailingConstraint = searchField.trailingAnchor.constraint(equalTo: searchContainer.trailingAnchor, constant: -12)
        inputLeadingConstraint = actionInputStack.leadingAnchor.constraint(equalTo: searchContainer.leadingAnchor, constant: 17)
        inlineQueryWidthConstraint = searchField.widthAnchor.constraint(equalToConstant: 1)
        inlineQueryWidthConstraint.priority = .defaultHigh
        inlineInputConstraints = [
            searchField.trailingAnchor.constraint(equalTo: actionInputStack.leadingAnchor, constant: -12),
            inlineQueryWidthConstraint,
        ]
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
            searchTrailingConstraint,
            searchField.centerYAnchor.constraint(equalTo: searchContainer.centerYAnchor),
            inputLeadingConstraint,
            actionInputStack.trailingAnchor.constraint(lessThanOrEqualTo: searchContainer.trailingAnchor, constant: -12),
            actionInputStack.centerYAnchor.constraint(equalTo: searchContainer.centerYAnchor),
            actionInputStack.heightAnchor.constraint(equalToConstant: 38),
            presentationTopConstraint!,
            presentationContainer.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: Layout.outerInset),
            presentationContainer.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -Layout.outerInset),
            presentationHeightConstraint!,
            presentationStack.topAnchor.constraint(equalTo: presentationContainer.topAnchor),
            presentationStack.leadingAnchor.constraint(equalTo: presentationContainer.leadingAnchor),
            presentationStack.trailingAnchor.constraint(equalTo: presentationContainer.trailingAnchor),
            presentationStack.bottomAnchor.constraint(equalTo: presentationContainer.bottomAnchor),
            contentTopConstraint!,
            content.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: Layout.outerInset),
            content.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -Layout.outerInset),
            content.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -Layout.outerInset),
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
        textPreview.isEditable = false
        textPreview.isSelectable = true
        textPreview.font = .systemFont(ofSize: 15)
        textPreview.drawsBackground = false
        textPreview.textContainerInset = NSSize(width: 12, height: 12)
        textPreview.isVerticallyResizable = true
        textPreview.autoresizingMask = [.width]
        textPreview.textContainer?.widthTracksTextView = true
        textPreviewScroll.documentView = textPreview
        textPreviewScroll.hasVerticalScroller = true
        textPreviewScroll.drawsBackground = false
        textPreviewScroll.isHidden = true
        textPreviewScroll.translatesAutoresizingMaskIntoConstraints = false
        detailPane.addSubview(textPreviewScroll)
        NSLayoutConstraint.activate([
            textPreviewScroll.leadingAnchor.constraint(equalTo: detailPane.leadingAnchor, constant: 8),
            textPreviewScroll.trailingAnchor.constraint(equalTo: detailPane.trailingAnchor, constant: -8),
            textPreviewScroll.topAnchor.constraint(equalTo: detailPane.topAnchor, constant: 8),
            textPreviewScroll.bottomAnchor.constraint(equalTo: detailPane.bottomAnchor, constant: -8),
        ])

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
        footerHeightConstraint = footer.heightAnchor.constraint(equalToConstant: 0)
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
            footerHeightConstraint!,
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
            self.updatePresentationItems()
            self.updateInlineArguments()
        }
        coordinator.onStatusChanged = { [weak self] status in self?.statusLabel.stringValue = status }
        coordinator.onPluginStateChanged = { [weak self] in self?.onPluginStateChanged?() }
    }

    func controlTextDidChange(_ obj: Notification) {
        guard let changedField = obj.object as? NSTextField, changedField === searchField else { return }
        let query = searchField.stringValue
        endActionInputForm(restoreSearchFocus: false)
        requestSearch(query)
        setExpanded(shouldExpand(for: query), animated: true)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        if activeActionInput != nil, control !== searchField {
            switch commandSelector {
            case #selector(NSResponder.insertTab(_:)):
                moveActionInputFocus(from: control, offset: 1)
                return true
            case #selector(NSResponder.insertBacktab(_:)):
                moveActionInputFocus(from: control, offset: -1)
                return true
            case #selector(NSResponder.insertNewline(_:)):
                submitActionInputForm()
                return true
            case #selector(NSResponder.cancelOperation(_:)):
                endActionInputForm(restoreSearchFocus: true)
                return true
            default:
                break
            }
        }
        switch commandSelector {
        case #selector(NSResponder.insertTab(_:)):
            requestActionInputForm(focusLastField: false); return true
        case #selector(NSResponder.insertBacktab(_:)):
            requestActionInputForm(focusLastField: true); return true
        case #selector(NSResponder.moveDown(_:)):
            moveSelection(1); return true
        case #selector(NSResponder.moveUp(_:)):
            moveSelection(-1); return true
        case #selector(NSResponder.insertNewline(_:)):
            coordinator.activateWhenSearchSettles { [weak self] in
                guard let self, self.isPresented else { return }
                self.openSelected()
            }
            return true
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
        case .plugins, .clipboard:
            false
        case let .plugin(plugin):
            plugin.id != "io.clipkeep.kill-process"
        case .global:
            true
        }
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        tableView.reloadData(forRowIndexes: IndexSet(integersIn: 0..<results.count), columnIndexes: IndexSet(integer: 0))
        updateModeLayout()
        updateDetails()
        updateInlineArguments()
    }

    @objc private func openSelected() {
        guard let result = selectedResult else { return }
        if result.metadata["route_mode"] == "plugins" {
            switchSearchMode(to: .plugins)
        } else if result.metadata["route_mode"] == "settings" {
            hide { [weak self] in self?.onShowSettings?() }
        } else if result.kind == .command,
           let pluginId = result.metadata["route_plugin_id"],
           coordinator.isEnabled(pluginId),
           let plugin = coordinator.descriptors.first(where: { $0.id == pluginId }) {
            switchSearchMode(to: pluginId == "io.clipkeep.clipboard" ? .clipboard : .plugin(plugin))
        } else if let action = result.actions.first(where: { $0.default }) ?? result.actions.first {
            perform(action, on: result)
        }
    }

    private func perform(_ action: ResultAction, on result: SearchResult) {
        coordinator.cancelPendingActivation()
        guard coordinator.canExecute(result) else { return }
        if !action.inputFields.isEmpty {
            beginActionInputForm(for: result, action: action)
            return
        }
        guard let descriptor = coordinator.descriptors.first(where: { $0.id == result.pluginId }),
              coordinator.isEnabled(result.pluginId) else { return }
        do {
            switch try PluginHostAction.resolve(result: result, actionId: action.id, descriptor: descriptor) {
            case .editText:
                performClipboardEdit(on: result, actionId: action.id)
                return
            case .notificationPermission:
                break
            case .menuPress:
                performMenuBarPress(on: result, actionId: action.id)
                return
            case .media, nil: break
            }
        } catch {
            coordinator.onStatusChanged?(error.localizedDescription)
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

    private func beginActionInputForm(
        for result: SearchResult,
        action: ResultAction,
        focusLastField: Bool = false,
        inline: Bool = false,
        takeFocus: Bool = true
    ) {
        if let activeActionInput, activeActionInput.result.id == result.id,
           activeActionInput.result.pluginId == result.pluginId,
           activeActionInput.action.id == action.id {
            if takeFocus, let field = focusLastField ? activeActionInput.fields.last?.1 : activeActionInput.fields.first?.1 {
                window?.makeFirstResponder(field)
            }
            return
        }
        endActionInputForm(restoreSearchFocus: false)
        inlineActionInput = inline
        searchIcon.isHidden = !inline
        searchField.isHidden = !inline
        actionInputStack.isHidden = false
        if inline {
            let textWidth = (searchField.stringValue as NSString).size(withAttributes: [
                .font: searchField.font ?? NSFont.systemFont(ofSize: 24),
            ]).width
            // Account for the text field's inset while keeping arguments beside the query.
            inlineQueryWidthConstraint.constant = ceil(textWidth) + 6
            searchTrailingConstraint.isActive = false
            inputLeadingConstraint.isActive = false
            NSLayoutConstraint.activate(inlineInputConstraints)
        }

        if !inline {
            let icon = NSImageView()
            icon.image = NSImage(
                systemSymbolName: result.metadata["symbol_name"] ?? "command",
                accessibilityDescription: result.title
            )
            icon.contentTintColor = .secondaryLabelColor
            icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 18, weight: .medium)
            icon.translatesAutoresizingMaskIntoConstraints = false
            icon.widthAnchor.constraint(equalToConstant: 24).isActive = true
            icon.heightAnchor.constraint(equalToConstant: 24).isActive = true
            actionInputStack.addArrangedSubview(icon)

            let title = NSTextField(labelWithString: result.title)
            title.font = .systemFont(ofSize: 16, weight: .semibold)
            title.textColor = .labelColor
            title.lineBreakMode = .byTruncatingTail
            title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            actionInputStack.addArrangedSubview(title)
        }

        var controls: [(ActionInputField, NSTextField)] = []
        for definition in action.inputFields {
            let (container, field) = makeActionInputField(definition)
            actionInputStack.addArrangedSubview(container)
            controls.append((definition, field))
        }
        for index in controls.indices {
            controls[index].1.nextKeyView = controls[controls.index(after: index) % controls.count].1
        }
        activeActionInput = ActiveActionInput(result: result, action: action, fields: controls)
        if takeFocus, let field = focusLastField ? controls.last?.1 : controls.first?.1 {
            window?.makeFirstResponder(field)
        }
    }

    private func makeActionInputField(_ definition: ActionInputField) -> (NSView, NSTextField) {
        let container = NSView()
        container.wantsLayer = true
        container.layer?.cornerRadius = 10
        container.layer?.cornerCurve = .continuous
        container.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.10).cgColor
        container.translatesAutoresizingMaskIntoConstraints = false
        container.widthAnchor.constraint(equalToConstant: definition.kind == .text ? 220 : 104).isActive = true
        container.heightAnchor.constraint(equalToConstant: 36).isActive = true

        let field = NSTextField()
        field.placeholderString = definition.placeholder.isEmpty ? definition.label : definition.placeholder
        field.font = .systemFont(ofSize: 15, weight: .medium)
        field.textColor = .labelColor
        field.focusRingType = .none
        field.isBezeled = false
        field.drawsBackground = false
        field.usesSingleLineMode = true
        field.delegate = self
        field.setAccessibilityLabel(definition.label)
        field.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(field)
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 11),
            field.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),
            field.centerYAnchor.constraint(equalTo: container.centerYAnchor),
        ])
        return (container, field)
    }

    private func endActionInputForm(restoreSearchFocus: Bool) {
        if inlineActionInput {
            NSLayoutConstraint.deactivate(inlineInputConstraints)
            searchTrailingConstraint.isActive = true
            inputLeadingConstraint.isActive = true
        }
        inlineActionInput = false
        activeActionInput = nil
        for view in actionInputStack.arrangedSubviews {
            actionInputStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        actionInputStack.isHidden = true
        searchIcon.isHidden = false
        searchField.isHidden = false
        if restoreSearchFocus {
            window?.makeFirstResponder(searchField)
        }
    }

    private func moveActionInputFocus(from control: NSControl, offset: Int) {
        guard let activeActionInput,
              let current = activeActionInput.fields.firstIndex(where: { $0.1 === control })
        else { return }
        let count = activeActionInput.fields.count
        let next = (current + offset + count) % count
        window?.makeFirstResponder(activeActionInput.fields[next].1)
    }

    private func submitActionInputForm() {
        guard let activeActionInput else { return }
        var values: [String: Any] = [:]
        for (definition, field) in activeActionInput.fields {
            let value = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if value.isEmpty {
                if definition.required {
                    statusLabel.stringValue = "Enter \(definition.label.lowercased())"
                    window?.makeFirstResponder(field)
                    return
                }
                if definition.kind == .text {
                    values[definition.id] = ""
                }
                continue
            }
            switch definition.kind {
            case .text:
                values[definition.id] = value
            case .integer:
                guard let number = Int(value),
                      definition.minimum.map({ number >= $0 }) ?? true,
                      definition.maximum.map({ number <= $0 }) ?? true
                else {
                    let range = [definition.minimum, definition.maximum]
                        .compactMap { $0 }
                        .map(String.init)
                        .joined(separator: "–")
                    statusLabel.stringValue = range.isEmpty
                        ? "Enter a valid \(definition.label.lowercased()) value"
                        : "\(definition.label) must be \(range)"
                    window?.makeFirstResponder(field)
                    return
                }
                values[definition.id] = number
            }
        }
        guard let data = try? JSONSerialization.data(withJSONObject: values),
              let input = String(data: data, encoding: .utf8)
        else { return }
        guard let descriptor = coordinator.descriptors.first(where: { $0.id == activeActionInput.result.pluginId }),
              coordinator.isEnabled(activeActionInput.result.pluginId) else { return }
        let hostAction: PluginHostAction?
        do {
            hostAction = try PluginHostAction.resolve(
                result: activeActionInput.result,
                actionId: activeActionInput.action.id,
                descriptor: descriptor
            )
        } catch {
            statusLabel.stringValue = error.localizedDescription
            return
        }
        if hostAction == .notificationPermission {
            SystemNotificationPresenter.shared.requestAuthorization(willPrompt: { [weak self] in
                self?.ignoreNextResign = true
            }) { [weak self] authorized in
                guard let self else { return }
                self.ignoreNextResign = false
                guard self.isPresented,
                      let current = self.activeActionInput,
                      current.result.id == activeActionInput.result.id,
                      current.result.pluginId == activeActionInput.result.pluginId,
                      current.action.id == activeActionInput.action.id,
                      self.coordinator.canExecute(current.result) else { return }
                self.window?.makeKey()
                guard authorized else {
                    self.statusLabel.stringValue = "Allow Nexus in System Settings > Notifications to create reminders"
                    return
                }
                self.executeActionInputForm(current, input: input)
            }
        } else {
            executeActionInputForm(activeActionInput, input: input)
        }
    }

    private func executeActionInputForm(_ activeActionInput: ActiveActionInput, input: String) {
        coordinator.execute(
            activeActionInput.result,
            actionId: activeActionInput.action.id,
            input: input
        ) { [weak self] success in
            guard let self, success else { return }
            let closesWindow = activeActionInput.action.closesWindow
            self.endActionInputForm(restoreSearchFocus: !closesWindow)
            if closesWindow {
                self.hide(restorePreviousApplication: true)
            }
        }
    }

    private func performMenuBarPress(on result: SearchResult, actionId: String) {
        guard ensureAccessibilityPermission(),
              let processIdentifier = result.hostParameter("target_pid", actionId: actionId).flatMap(Int32.init),
              let encodedPath = result.hostParameter("menu_path", actionId: actionId)?.data(using: .utf8),
              let menuPath = try? JSONDecoder().decode([String].self, from: encodedPath)
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
            self?.performMenuBarCommand(
                path: menuPath,
                in: application,
                attemptsRemaining: 20
            )
        }
    }

    private func performMenuBarCommand(
        path: [String],
        in application: NSRunningApplication,
        attemptsRemaining: Int
    ) {
        let processIdentifier = application.processIdentifier
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == processIdentifier else {
            guard attemptsRemaining > 0 else {
                NSLog(
                    "Menu Bar: target application did not become frontmost: pid=%d path=%@",
                    processIdentifier,
                    path.joined(separator: " > ")
                )
                coordinator.onStatusChanged?("The target application did not become active")
                return
            }
            application.activate()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
                self?.performMenuBarCommand(
                    path: path,
                    in: application,
                    attemptsRemaining: attemptsRemaining - 1
                )
            }
            return
        }

        performMenuProviderRetry(
            path: path,
            in: application,
            attemptsRemaining: attemptsRemaining
        )
    }

    private func performMenuProviderRetry(
        path: [String],
        in application: NSRunningApplication,
        attemptsRemaining: Int
    ) {
        let processIdentifier = application.processIdentifier
        switch menuBarProvider.pressItem(processIdentifier: processIdentifier, path: path) {
        case .success:
            return
        case .menuNotFound where attemptsRemaining > 0:
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
                self?.performMenuBarCommand(
                    path: path,
                    in: application,
                    attemptsRemaining: attemptsRemaining - 1
                )
            }
        case .menuNotFound:
            NSLog(
                "Menu Bar: command was not found after activation: pid=%d path=%@",
                processIdentifier,
                path.joined(separator: " > ")
            )
            coordinator.onStatusChanged?("The selected menu command is no longer available")
        case .actionFailed where attemptsRemaining > 0:
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
                self?.performMenuBarCommand(
                    path: path,
                    in: application,
                    attemptsRemaining: attemptsRemaining - 1
                )
            }
        case let .actionFailed(error):
            NSLog(
                "Menu Bar: AXPress failed: pid=%d error=%d path=%@",
                processIdentifier,
                error.rawValue,
                path.joined(separator: " > ")
            )
            coordinator.onStatusChanged?("The target application rejected the menu command")
        }
    }

    private func performClipboardEdit(on result: SearchResult, actionId: String) {
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
                    coordinator.execute(result, actionId: result.hostParameter("edit_submit_action", actionId: actionId) ?? "apply_edit", input: editedText) { success in
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
        endActionInputForm(restoreSearchFocus: false)
        searchMode = mode
        searchField.stringValue = ""
        searchField.placeholderString = mode.placeholder
        statusLabel.stringValue = mode.title
        updateModeLayout()
        updatePresentationItems()
        refreshMenuBarPlugin(for: mode)
        requestSearch("", preservingResults: true)
        setExpanded(shouldExpand(for: ""), animated: true)
        window?.makeFirstResponder(searchField)
    }

    private func refreshMenuBarPlugin(for mode: SearchMode) {
        let targets = coordinator.descriptors.filter { plugin in
            guard plugin.capabilities.contains("accessibility.read"), coordinator.isEnabled(plugin.id) else { return false }
            switch mode {
            case .global: return coordinator.isIncludedInGlobalSearch(plugin.id)
            case let .plugin(selected): return selected.id == plugin.id
            case .plugins, .clipboard: return false
            }
        }
        guard !targets.isEmpty, ensureAccessibilityPermission(), let previousApplication else { return }
        let items = menuBarProvider.items(for: previousApplication)
        guard let data = try? JSONEncoder().encode(items),
              let encodedItems = String(data: data, encoding: .utf8)
        else { return }
        for plugin in targets {
            coordinator.configureRuntimeSettings([
                "target_pid": String(previousApplication.processIdentifier),
                "items": encodedItems,
            ], for: plugin.id)
        }
    }

    private var selectedResult: SearchResult? {
        guard results.indices.contains(tableView.selectedRow) else { return nil }
        return results[tableView.selectedRow]
    }

    private func moveSelection(_ delta: Int) {
        coordinator.cancelPendingActivation()
        guard !results.isEmpty else { return }
        let current = max(tableView.selectedRow, 0)
        let next = min(max(current + delta, 0), results.count - 1)
        tableView.selectRowIndexes(IndexSet(integer: next), byExtendingSelection: false)
        tableView.scrollRowToVisible(next)
    }

    private func moveSelectionByHalfPage(_ direction: Int) {
        let rowStride = max(tableView.rowHeight + tableView.intercellSpacing.height, 1)
        let visibleRows = max(Int(scrollView.contentView.bounds.height / rowStride), 1)
        moveSelection(direction * max(visibleRows / 2, 1))
    }

    /// Preview the first command's arguments without taking focus from the query.
    private func updateInlineArguments() {
        guard activeActionInput == nil || inlineActionInput else { return }
        let isRemindersPage: Bool
        if case let .plugin(plugin) = searchMode {
            isRemindersPage = plugin.id == "io.clipkeep.reminders"
        } else {
            isRemindersPage = false
        }
        guard (searchMode == .global || isRemindersPage), !searchField.stringValue.isEmpty,
              tableView.selectedRow == 0, let result = results.first,
              (!isRemindersPage || result.id == "reminder:new"),
              coordinator.canExecute(result),
              let action = result.actions.first(where: { $0.default }) ?? result.actions.first,
              !action.inputFields.isEmpty else {
            if inlineActionInput { endActionInputForm(restoreSearchFocus: true) }
            return
        }
        if let activeActionInput, !coordinator.canExecute(activeActionInput.result) {
            endActionInputForm(restoreSearchFocus: true)
        }
        beginActionInputForm(for: result, action: action, inline: true, takeFocus: false)
    }

    /// Uses the same settled-search gate as Return so Tab cannot bind stale arguments.
    private func requestActionInputForm(focusLastField: Bool) {
        coordinator.activateWhenSearchSettles { [weak self] in
            guard let self, self.isPresented,
                  let result = self.selectedResult,
                  self.coordinator.canExecute(result),
                  let action = result.actions.first(where: { $0.default }) ?? result.actions.first,
                  !action.inputFields.isEmpty else { return }
            self.beginActionInputForm(for: result, action: action, focusLastField: focusLastField)
        }
    }

    private func handleKeyEvent(_ event: NSEvent) -> Bool {
        let relevantModifiers = event.modifierFlags.intersection([.command, .option, .control, .shift])
        if event.keyCode == 48,
           relevantModifiers.subtracting(.shift).isEmpty {
            let offset = relevantModifiers.contains(.shift) ? -1 : 1
            if searchField.currentEditor() === window?.firstResponder {
                requestActionInputForm(focusLastField: offset < 0)
                return true
            }
            if let activeActionInput {
                let currentEditor = window?.firstResponder
                if let current = activeActionInput.fields.firstIndex(where: {
                    $0.1.currentEditor() === currentEditor
                }) {
                    let count = activeActionInput.fields.count
                    let next = (current + offset + count) % count
                    window?.makeFirstResponder(activeActionInput.fields[next].1)
                } else if let field = offset < 0
                    ? activeActionInput.fields.last?.1
                    : activeActionInput.fields.first?.1 {
                    window?.makeFirstResponder(field)
                }
                return true
            }

        }
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
        if LauncherKeyAction.moveHalfPageDown.matches(event) {
            moveSelectionByHalfPage(1)
            return true
        }
        if LauncherKeyAction.moveHalfPageUp.matches(event) {
            moveSelectionByHalfPage(-1)
            return true
        }
        return false
    }

    private func handle(_ selector: Selector) -> Bool {
        if selector == NSSelectorFromString("showSettings:") {
            onShowSettings?()
            return true
        }
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
        let hasTextPreview = selectedResult?.metadata["preview_style"] == "text"
        textPreviewScroll.isHidden = !hasTextPreview
        detailInfo.isHidden = hasTextPreview
        if hasTextPreview {
            detailImageGeneration += 1
            detailText.isHidden = true
            detailImage.isHidden = true
            let text = selectedResult?.preview ?? ""
            if textPreview.string != text {
                textPreview.string = text
                textPreview.scrollToBeginningOfDocument(nil)
            }
            return
        }
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
                !$0.key.hasPrefix("_nexus_") && !$0.key.hasPrefix("host_action.") && ![
                    "edit_submit_action",
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

    private var showsResultDetails: Bool {
        searchMode.showsDetails || selectedResult?.metadata["preview_style"] == "text"
    }

    private func updateModeLayout() {
        enforcePanelWidth()
        let showsDetails = showsResultDetails
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
        case .plugins:
            coordinator.searchPluginEntries(query, preservingPublishedResults: preservingResults)
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
        showsResultDetails ? proposedEffectiveRect : .zero
    }

    func splitView(_ splitView: NSSplitView, shouldHideDividerAt dividerIndex: Int) -> Bool {
        !showsResultDetails
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

    private func updatePresentationItems() {
        let items: [PluginPresentationItem]
        if case let .plugin(plugin) = searchMode {
            items = Array(coordinator.presentationItems(for: plugin.id).prefix(3))
        } else {
            items = []
        }

        for view in presentationStack.arrangedSubviews {
            presentationStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        presentationValueLabels.removeAll()
        presentationTimer?.invalidate()
        presentationTimer = nil

        for item in items {
            presentationStack.addArrangedSubview(makePresentationRow(item))
        }
        let visible = !items.isEmpty
        let height = visible
            ? CGFloat(items.count * 36 + max(items.count - 1, 0) * 6)
            : 0
        let layoutChanged = presentationContainer.isHidden == visible
            || presentationTopConstraint?.constant != (visible ? Layout.contentSpacing : 0)
            || presentationHeightConstraint?.constant != height
        presentationContainer.isHidden = !visible
        presentationTopConstraint?.constant = visible ? Layout.contentSpacing : 0
        presentationHeightConstraint?.constant = height
        if layoutChanged {
            root.layoutSubtreeIfNeeded()
        }
        updateScrollerVisibility()

        if items.contains(where: { $0.metadata["countdown_until"] != nil }) {
            let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
                self?.refreshPresentationValues()
            }
            presentationTimer = timer
            RunLoop.main.add(timer, forMode: .common)
        }
    }

    private func makePresentationRow(_ item: PluginPresentationItem) -> NSView {
        let row = NSStackView()
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 8
        row.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)
        row.wantsLayer = true
        row.layer?.cornerRadius = 11
        row.layer?.cornerCurve = .continuous
        row.layer?.backgroundColor = presentationColor(for: item.style).cgColor
        row.heightAnchor.constraint(equalToConstant: 36).isActive = true

        let icon = NSImageView()
        let symbolName = item.metadata["symbol_name"] ?? "info.circle.fill"
        icon.image = NSImage(systemSymbolName: symbolName, accessibilityDescription: item.title)
        icon.contentTintColor = .secondaryLabelColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .medium)
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.widthAnchor.constraint(equalToConstant: 18).isActive = true
        icon.heightAnchor.constraint(equalToConstant: 18).isActive = true

        let title = NSTextField(labelWithString: item.title)
        title.font = .systemFont(ofSize: 13, weight: .medium)
        title.textColor = .labelColor
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let value = NSTextField(labelWithString: presentationValue(for: item))
        value.font = .monospacedDigitSystemFont(ofSize: 13, weight: .medium)
        value.textColor = .secondaryLabelColor
        value.alignment = .right
        value.setContentHuggingPriority(.required, for: .horizontal)
        value.setContentCompressionResistancePriority(.required, for: .horizontal)

        row.addArrangedSubview(icon)
        row.addArrangedSubview(title)
        row.addArrangedSubview(value)
        presentationValueLabels.append((value, item))
        return row
    }

    private func refreshPresentationValues() {
        for (label, item) in presentationValueLabels {
            label.stringValue = presentationValue(for: item)
        }
    }

    private func presentationValue(for item: PluginPresentationItem) -> String {
        guard let rawTarget = item.metadata["countdown_until"],
              let target = TimeInterval(rawTarget)
        else { return item.value }
        let remaining = max(Int(ceil(target - Date().timeIntervalSince1970)), 0)
        let hours = remaining / 3_600
        let minutes = (remaining % 3_600) / 60
        let seconds = remaining % 60
        if hours > 0 {
            return String(format: "%d:%02d:%02d remaining", hours, minutes, seconds)
        }
        return String(format: "%02d:%02d remaining", minutes, seconds)
    }

    private func presentationColor(for style: PluginPresentationStyle) -> NSColor {
        switch style {
        case .info: NSColor.controlAccentColor.withAlphaComponent(0.10)
        case .success: NSColor.systemGreen.withAlphaComponent(0.11)
        case .warning: NSColor.systemOrange.withAlphaComponent(0.12)
        }
    }

    private func setExpanded(_ shouldExpand: Bool, animated: Bool) {
        guard expanded != shouldExpand || !animated else { return }
        expanded = shouldExpand
        guard let panel = window else { return }
        expansionAnimationGeneration &+= 1
        let generation = expansionAnimationGeneration
        var frame = panel.frame
        let newHeight = shouldExpand ? Layout.expandedHeight : Layout.collapsedHeight
        frame.origin.y += frame.height - newHeight
        frame.size.height = newHeight

        guard animated else {
            contentTopConstraint?.constant = shouldExpand ? Layout.contentSpacing : 0
            footerHeightConstraint?.constant = shouldExpand ? 46 : 0
            content.alphaValue = shouldExpand ? 1 : 0
            presentationContainer.alphaValue = shouldExpand ? 1 : 0
            content.isHidden = !shouldExpand
            panel.setFrame(frame, display: true)
            root.layoutSubtreeIfNeeded()
            updateScrollerVisibility()
            return
        }

        if shouldExpand {
            // Keep result views out of the live glass resize. AppKit otherwise lays
            // out the table while NSGlassEffectView is resampling its background,
            // which produces a visible one-frame jump on the first character.
            content.alphaValue = 0
            content.isHidden = true
            presentationContainer.alphaValue = 0
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.18
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrame(frame, display: true)
            }, completionHandler: { [weak self] in
                guard let self, self.expansionAnimationGeneration == generation else { return }
                self.contentTopConstraint?.constant = Layout.contentSpacing
                self.footerHeightConstraint?.constant = 46
                self.root.layoutSubtreeIfNeeded()
                self.content.isHidden = false
                NSAnimationContext.runAnimationGroup({ context in
                    context.duration = 0.08
                    context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                    self.content.animator().alphaValue = 1
                    self.presentationContainer.animator().alphaValue = 1
                }, completionHandler: { [weak self] in
                    guard let self, self.expansionAnimationGeneration == generation else { return }
                    self.updateScrollerVisibility()
                })
            })
        } else {
            content.alphaValue = 0
            presentationContainer.alphaValue = 0
            content.isHidden = true
            contentTopConstraint?.constant = 0
            footerHeightConstraint?.constant = 0
            root.layoutSubtreeIfNeeded()
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.24
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrame(frame, display: true)
            }, completionHandler: { [weak self] in
                guard let self, self.expansionAnimationGeneration == generation else { return }
                self.updateScrollerVisibility()
            })
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

    func controlTextDidBeginEditing(_ notification: Notification) {
        guard notification.object as? NSTextField === searchField else { return }
        refreshSearchCaret()
    }

    @objc private func refreshSearchCaret() {
        guard window?.isKeyWindow == true,
              let editor = searchField.currentEditor() as? NSTextView,
              window?.firstResponder === editor else { return }
        // Use AppKit's insertion point so blinking follows keyboard focus,
        // selection, and input-method composition rather than a separate timer.
        editor.insertionPointColor = .labelColor
        editor.updateInsertionPointStateAndRestartTimer(true)
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
        artwork.contentTintColor = resultSymbolColor(from: result.metadata["symbol_color"]) ?? .labelColor
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
        guard (result.kind == .application || result.kind == .file),
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

func resultSymbolColor(from value: String?) -> NSColor? {
    guard let value, value.hasPrefix("#") else { return nil }
    let hex = String(value.dropFirst())
    guard (hex.count == 6 || hex.count == 8), let number = UInt64(hex, radix: 16) else {
        return nil
    }
    let hasAlpha = hex.count == 8
    let red = hasAlpha ? (number >> 24) & 0xff : (number >> 16) & 0xff
    let green = hasAlpha ? (number >> 16) & 0xff : (number >> 8) & 0xff
    let blue = hasAlpha ? (number >> 8) & 0xff : number & 0xff
    let alpha = hasAlpha ? number & 0xff : 0xff
    return NSColor(
        srgbRed: CGFloat(red) / 255,
        green: CGFloat(green) / 255,
        blue: CGFloat(blue) / 255,
        alpha: CGFloat(alpha) / 255
    )
}
