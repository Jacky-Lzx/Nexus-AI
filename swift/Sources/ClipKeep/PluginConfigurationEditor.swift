import AppKit

/// A scrollable form shared by independently installed plugins.
enum PluginConfigurationEditor {
    static func present(name: String, fields: [PluginConfigurationField], settings: [String: String], save: ([String: String]) -> Void) {
        let alert = NSAlert()
        alert.messageText = "\(name) Settings"
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Cancel")
        let document = NSView(frame: NSRect(x: 0, y: 0, width: 360, height: fields.count * 72))
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 380, height: min(360, fields.count * 72)))
        scroll.hasVerticalScroller = true
        scroll.documentView = document
        var controls: [(PluginConfigurationField, NSControl)] = []
        for (index, field) in fields.enumerated() {
            let y = CGFloat((fields.count - index - 1) * 72)
            let label = NSTextField(labelWithString: field.label)
            label.frame = NSRect(x: 0, y: y + 38, width: 350, height: 22)
            label.toolTip = field.description
            document.addSubview(label)
            let value = settings[field.id] ?? field.default.legacyString
            let control: NSControl
            switch field.kind {
            case "boolean":
                let toggle = NSSwitch()
                toggle.state = value == "true" ? .on : .off
                control = toggle
            case "choice":
                let popup = NSPopUpButton()
                popup.addItems(withTitles: field.options ?? [])
                popup.selectItem(withTitle: value)
                control = popup
            default:
                let text = NSTextField(string: value)
                text.placeholderString = field.kind == "integer" ? "Integer" : nil
                control = text
            }
            control.frame = NSRect(x: 0, y: y + 6, width: field.kind == "boolean" ? 50 : 350, height: 26)
            document.addSubview(control)
            controls.append((field, control))
        }
        alert.accessoryView = scroll
        while alert.runModal() == .alertFirstButtonReturn {
            var values: [String: String] = [:]
            var invalid: String?
            for (field, control) in controls {
                let text: String
                if let toggle = control as? NSSwitch { text = toggle.state == .on ? "true" : "false" }
                else if let popup = control as? NSPopUpButton { text = popup.titleOfSelectedItem ?? "" }
                else { text = control.stringValue }
                guard let value = field.parse(text) else { invalid = field.label; break }
                values[field.id] = value.legacyString
            }
            if let invalid { alert.informativeText = "Enter a valid value for \(invalid)." }
            else { save(settings.merging(values) { _, new in new }); return }
        }
    }
}
