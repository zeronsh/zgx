// AppKit's own, for the same hover and clicks as zgx's popup_hover example.
// ref --tip | --popover transient|semitransient|applicationDefined | --tracking
import AppKit
func say(_ s: String) { print(s); fflush(stdout) }
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let args = CommandLine.arguments
func window(_ title: String, _ r: NSRect) -> NSWindow {
    let w = NSWindow(contentRect: r, styleMask: [.titled], backing: .buffered, defer: false)
    w.title = title; w.isReleasedWhenClosed = false; return w
}
// `POPUP_HOVER_X`: points to move it across, to another display.
let dx = CGFloat(Double(ProcessInfo.processInfo.environment["POPUP_HOVER_X"] ?? "0") ?? 0)
let parent = window("ref parent", NSRect(x: 200 + dx, y: 300, width: 420, height: 300))
let other = window("ref other", NSRect(x: 640 + dx, y: 300, width: 200, height: 200))
let tip = NSView(frame: NSRect(x: 16, y: 256, width: 120, height: 28))
tip.toolTip = "A tooltip"
parent.contentView!.addSubview(tip)
func onScreen(_ w: NSWindow, _ x: CGFloat, _ yFromBottom: CGFloat) -> (CGFloat, CGFloat) {
    let c = w.contentRect(forFrameRect: w.frame)
    return (c.minX + x, NSScreen.screens[0].frame.height - (c.minY + yFromBottom))
}
var pop: NSPopover?
// (Set again now it exists: made there, AppKit puts a window on the main display.)
parent.setFrameOrigin(NSPoint(x: 200 + dx, y: 300))
other.setFrameOrigin(NSPoint(x: 640 + dx, y: 300))
parent.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
if let i = args.firstIndex(of: "--popover") {
    other.orderFront(nil)
    let p = NSPopover(); let c = NSViewController(); c.view = NSView(frame: NSRect(x: 0, y: 0, width: 120, height: 80))
    p.contentViewController = c
    p.behavior = ["transient": .transient, "semitransient": .semitransient][args[i + 1]] ?? .applicationDefined
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { p.show(relativeTo: tip.bounds, of: tip, preferredEdge: .minY) }
    pop = p
}
// What reaches a view at the back, by how it asks: a tracking area's options,
// the legacy tracking rect winit's view uses, a real NSButton, plain mouseMoved.
final class Heard: NSView {
    var name = ""
    override func mouseEntered(with e: NSEvent) { say("HEARD \(name) entered active=\(app.isActive) key=\(window?.isKeyWindow ?? false)") }
    override func mouseExited(with e: NSEvent) { say("HEARD \(name) exited") }
    override func mouseMoved(with e: NSEvent) { say("HEARD \(name) moved active=\(app.isActive) key=\(window?.isKeyWindow ?? false)") }
}
final class HeardButton: NSButton {
    override func mouseEntered(with e: NSEvent) { say("HEARD button entered active=\(app.isActive) key=\(window?.isKeyWindow ?? false)"); super.mouseEntered(with: e) }
    override func mouseExited(with e: NSEvent) { say("HEARD button exited"); super.mouseExited(with: e) }
}
var tracked: [(String, NSView)] = []
if args.contains("--tracking") {
    parent.acceptsMouseMovedEvents = true
    let kinds: [(String, NSTrackingArea.Options)] = [("always", .activeAlways), ("activeApp", .activeInActiveApp), ("keyWindow", .activeInKeyWindow)]
    for (i, kind) in kinds.enumerated() {
        let v = Heard(frame: NSRect(x: 16 + CGFloat(i) * 76, y: 256, width: 70, height: 28)); v.name = kind.0
        v.addTrackingArea(NSTrackingArea(rect: v.bounds, options: [.mouseEnteredAndExited, .mouseMoved, kind.1], owner: v, userInfo: nil))
        parent.contentView!.addSubview(v); tracked.append((kind.0, v))
    }
    let legacy = Heard(frame: NSRect(x: 244, y: 256, width: 70, height: 28)); legacy.name = "legacyRect"
    parent.contentView!.addSubview(legacy)
    legacy.addTrackingRect(legacy.bounds, owner: legacy, userData: nil, assumeInside: false)
    tracked.append(("legacyRect", legacy))
    let b = HeardButton(frame: NSRect(x: 320, y: 256, width: 90, height: 28)); b.title = "Button"; b.bezelStyle = .rounded
    b.showsBorderOnlyWhileMouseInside = true
    parent.contentView!.addSubview(b); tracked.append(("button", b))
    if args.contains("--not-key") { other.makeKeyAndOrderFront(nil) }
}
// `POPUP_HOVER_ACT`: done to its window a second after a tooltip shows.
let act = ProcessInfo.processInfo.environment["POPUP_HOVER_ACT"]
var last = "", seen = 0, acted = false
Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { _ in
    if app.windows.contains(where: { $0.level.rawValue > 0 && $0.isVisible }) { seen += 1 }
    if let act, seen >= 10, !acted {
        acted = true; say("ACT \(act)")
        switch act {
        case "move": parent.setFrameOrigin(NSPoint(x: parent.frame.minX + 60, y: parent.frame.minY + 40))
        case "resize": var f = parent.frame; f.size.width += 80; f.size.height += 60; parent.setFrame(f, display: true)
        case "mini": parent.miniaturize(nil)
        case "full": parent.collectionBehavior.insert(.fullScreenPrimary); parent.toggleFullScreen(nil)
        default: break
        }
    }
    var line = "active=\(app.isActive) popover=\(pop?.isShown.description ?? "-")"
    for w in app.windows {
        line += " | \(type(of: w)) #\(w.windowNumber) level=\(w.level.rawValue) parent=\(w.parent?.windowNumber ?? 0) key=\(w.isKeyWindow) visible=\(w.isVisible) at=\(w.frame.minX),\(w.frame.minY)"
    }
    if line != last { say("STATE " + line); last = line }
}
let t = onScreen(parent, 76, 270); say("TARGET tip \(t.0) \(t.1)")
let o = onScreen(parent, 300, 40); say("TARGET outside \(o.0) \(o.1)")
let s = onScreen(other, 100, 100); say("TARGET other \(s.0) \(s.1)")
for (name, v) in tracked {
    let c = onScreen(parent, v.frame.midX, v.frame.midY); say("TARGET \(name) \(c.0) \(c.1)")
}
say("READY")
app.run()
