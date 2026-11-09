// Another app's window, frontmost, over the lower part of the target's.
// front x y width height (the screen's points, from its top left)
import AppKit
let a = CommandLine.arguments.dropFirst().map { CGFloat(Double($0)!) }
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let f = NSRect(x: a[0], y: NSScreen.screens[0].frame.height - a[1] - a[3], width: a[2], height: a[3])
let w = NSWindow(contentRect: f, styleMask: [.borderless], backing: .buffered, defer: false)
w.backgroundColor = .systemTeal; w.isReleasedWhenClosed = false
w.setFrameOrigin(f.origin)
w.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
print("FRONT up"); fflush(stdout)
app.run()
