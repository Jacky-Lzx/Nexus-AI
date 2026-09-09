import AppKit

if CommandLine.arguments.contains("--list-plugins") {
    for plugin in PluginRegistry.discover() {
        print("\(plugin.id)\t\(plugin.version)\t\(plugin.executableURL.path)")
    }
    exit(EXIT_SUCCESS)
}

let application = NSApplication.shared
let applicationDelegate = AppDelegate()
application.delegate = applicationDelegate
application.run()
