import AppKit

/// Presents plugin notifications as lightweight, non-activating in-app toasts.
final class NotificationPresenter {
    static let shared = NotificationPresenter()

    private let panel: NSPanel
    private let titleLabel = NSTextField(labelWithString: "")
    private let bodyLabel = NSTextField(wrappingLabelWithString: "")
    private var pending: [PluginNotification] = []
    private var isPresenting = false

    private init() {
        panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 68),
            styleMask: [.borderless, .nonactivatingPanel, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        configurePanel()
        buildInterface()
    }

    func deliver(_ notification: PluginNotification) {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { [weak self] in self?.deliver(notification) }
            return
        }
        pending.append(notification)
        presentNextIfNeeded()
    }

    private func configurePanel() {
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.ignoresMouseEvents = true
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.animationBehavior = .none
    }

    private func buildInterface() {
        let background = NSVisualEffectView()
        background.material = .hudWindow
        background.blendingMode = .behindWindow
        background.state = .active
        background.wantsLayer = true
        background.layer?.cornerRadius = 15
        background.layer?.cornerCurve = .continuous
        background.translatesAutoresizingMaskIntoConstraints = false

        let icon = NSImageView()
        icon.image = NSImage(
            systemSymbolName: "checkmark.circle.fill",
            accessibilityDescription: "Success"
        )
        icon.contentTintColor = .systemGreen
        icon.symbolConfiguration = .init(pointSize: 20, weight: .semibold)
        icon.translatesAutoresizingMaskIntoConstraints = false

        titleLabel.font = .systemFont(ofSize: 13, weight: .semibold)
        titleLabel.textColor = .labelColor
        titleLabel.lineBreakMode = .byTruncatingTail

        bodyLabel.font = .systemFont(ofSize: 12)
        bodyLabel.textColor = .secondaryLabelColor
        bodyLabel.maximumNumberOfLines = 2
        bodyLabel.lineBreakMode = .byTruncatingTail

        let labels = NSStackView(views: [titleLabel, bodyLabel])
        labels.orientation = .vertical
        labels.alignment = .leading
        labels.spacing = 2
        labels.translatesAutoresizingMaskIntoConstraints = false

        background.addSubview(icon)
        background.addSubview(labels)
        panel.contentView = background

        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: background.leadingAnchor, constant: 16),
            icon.centerYAnchor.constraint(equalTo: background.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 22),
            icon.heightAnchor.constraint(equalToConstant: 22),
            labels.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 11),
            labels.trailingAnchor.constraint(equalTo: background.trailingAnchor, constant: -16),
            labels.centerYAnchor.constraint(equalTo: background.centerYAnchor),
        ])
    }

    private func presentNextIfNeeded() {
        guard !isPresenting, !pending.isEmpty else { return }
        isPresenting = true
        let notification = pending.removeFirst()
        titleLabel.stringValue = notification.title
        bodyLabel.stringValue = notification.body
        positionPanel()

        panel.alphaValue = 0
        panel.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.18
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 2.4) { [weak self] in
            self?.dismissCurrent()
        }
    }

    private func dismissCurrent() {
        NSAnimationContext.runAnimationGroup({ [weak self] context in
            guard let self else { return }
            context.duration = 0.22
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            guard let self else { return }
            panel.orderOut(nil)
            isPresenting = false
            presentNextIfNeeded()
        })
    }

    private func positionPanel() {
        let mouseLocation = NSEvent.mouseLocation
        let screen = NSScreen.screens.first(where: { NSMouseInRect(mouseLocation, $0.frame, false) })
            ?? NSScreen.main
            ?? NSScreen.screens.first
        guard let visibleFrame = screen?.visibleFrame else { return }
        let frame = panel.frame
        panel.setFrameOrigin(NSPoint(
            x: visibleFrame.midX - frame.width / 2,
            y: visibleFrame.minY + visibleFrame.height * 0.16
        ))
    }
}
