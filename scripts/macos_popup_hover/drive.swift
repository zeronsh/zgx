// Runs each case with the real pointer. Before every posted event, whatever
// is frontmost at the point, of any layer, must be the target's (WindowServer's
// own cursor window aside, logged); otherwise it stops. Never touches a dialog.
// The one unguarded event puts the pointer back where it was, a move, no click.
// Someone at the machine stops it: before every target, the system's idle time
// must reach back to this run's own last posted event (`ZGX_HOVER_MIN_IDLE`
// minutes before the first); it asks nothing and restores the pointer.
// drive <dir with ref and front> <zgx popup_hover binary> [pop2]
import AppKit
import ApplicationServices
func say(_ s: String) { print(s); fflush(stdout) }
guard CGPreflightPostEventAccess() else { say("STOP: posting is not permitted; not asking"); exit(3) }
let dir = CommandLine.arguments[1], zgx = CommandLine.arguments[2]
let home = CGEvent(source: nil)!.location

var children: [Child] = []
final class Child {
    let p = Process(); var targets: [String: CGPoint] = [:]; var ready = false
    init(_ path: String, _ args: [String], tag: String) {
        p.executableURL = URL(fileURLWithPath: path); p.arguments = args
        let out = Pipe(); p.standardOutput = out; p.standardError = FileHandle.nullDevice
        out.fileHandleForReading.readabilityHandler = { h in
            guard let s = String(data: h.availableData, encoding: .utf8) else { return }
            for l in s.split(separator: "\n") {
                let w = l.split(separator: " ")
                if w.first == "TARGET", w.count == 4 { self.targets[String(w[1])] = CGPoint(x: Double(w[2])!, y: Double(w[3])!) }
                if w.first == "READY" { self.ready = true }
                say("  [\(tag) \(Date().timeIntervalSince1970)] \(l)")
            }
        }
        try! p.run()
        children.append(self)
    }
    func stop() { if p.isRunning { p.terminate(); p.waitUntilExit() } }
}
var lastPost: Date? = nil
func idleSeconds() -> Double {
    let p = Process(), out = Pipe()
    p.executableURL = URL(fileURLWithPath: "/usr/sbin/ioreg"); p.arguments = ["-c", "IOHIDSystem"]; p.standardOutput = out
    try! p.run()
    let text = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
    p.waitUntilExit()
    guard let r = text.range(of: #""HIDIdleTime" = \d+"#, options: .regularExpression) else { say("STOP: no HIDIdleTime"); finish(6) }
    return Double(text[r].split(separator: " ").last!)! / 1e9
}
func stillIdle() {
    let idle = idleSeconds()
    if let last = lastPost {
        // Our own events reset it: anything newer than them is a person's.
        let since = Date().timeIntervalSince(last)
        if idle + 0.5 < since { say("STOP: someone is at the machine: idle \(idle) s, our last event \(since) s ago"); finish(6) }
    } else {
        let least = (Double(ProcessInfo.processInfo.environment["ZGX_HOVER_MIN_IDLE"] ?? "0") ?? 0) * 60
        if idle < least { say("STOP: idle \(idle) s, under \(least) s"); finish(6) }
    }
    say("  (idle \(idle) s)")
}
func wait(_ s: Double) { RunLoop.main.run(until: Date().addingTimeInterval(s)) }
func windows() -> [[String: Any]] {
    CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
}
func order(_ pids: [Int32: String]) {
    let front = NSWorkspace.shared.frontmostApplication
    say("  frontmost app: \(front?.localizedName ?? "?") pid=\(front?.processIdentifier ?? 0)")
    for (i, w) in windows().enumerated() {
        guard let pid = w[kCGWindowOwnerPID as String] as? Int32, let name = pids[pid] else { continue }
        let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
        say("  z\(i) \(name) #\(w[kCGWindowNumber as String] ?? 0) layer=\(w[kCGWindowLayer as String] ?? 0) \(b["X"] ?? 0),\(b["Y"] ?? 0) \(b["Width"] ?? 0)x\(b["Height"] ?? 0)")
    }
}
func owns(_ at: CGPoint, _ pid: Int32) -> Bool {
    for w in windows() {
        guard let d = w[kCGWindowBounds as String] as? NSDictionary, let r = CGRect(dictionaryRepresentation: d), r.contains(at) else { continue }
        let owner = w[kCGWindowOwnerName as String] as? String ?? "", name = w[kCGWindowName as String] as? String ?? ""
        let layer = w[kCGWindowLayer as String] as? Int ?? 0
        if owner == "WindowServer", name == "Cursor", r.width <= 64, r.height <= 64 { say("  (guard skips the cursor window)"); continue }
        // Its own, of any layer: a popup it opened under the pointer.
        if w[kCGWindowOwnerPID as String] as? Int32 == pid { return true }
        say("STOP: at \(at) frontmost is \(owner) '\(name)' layer \(layer)"); return false
    }
    say("STOP: nothing at \(at)"); return false
}
func move(_ to: CGPoint, _ pid: Int32) -> Bool {
    stillIdle()
    // A jump, then small moves on the target: every event guarded, so it
    // never passes over anything but the target's window.
    for d in [CGFloat(-3), -2, -1, 0] {
        let at = CGPoint(x: to.x + d, y: to.y)
        if !owns(at, pid) { return false }
        CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: at, mouseButton: .left)!.post(tap: .cghidEventTap)
        lastPost = Date()
        wait(0.03)
    }
    return true
}
func click(_ at: CGPoint, _ pid: Int32) -> Bool {
    stillIdle()
    guard owns(at, pid) else { return false }
    lastPost = Date()
    for t in [CGEventType.leftMouseDown, .leftMouseUp] {
        CGEvent(mouseEventSource: nil, mouseType: t, mouseCursorPosition: at, mouseButton: .left)!.post(tap: .cghidEventTap); wait(0.05)
    }
    return true
}
func launch(_ path: String, _ args: [String], _ tag: String) -> Child {
    let c = Child(path, args, tag: tag); var n = 0
    while !c.ready && n < 300 { wait(0.1); n += 1 }
    if !c.ready { say("STOP: \(tag) never ready"); c.stop(); finish(4) }
    return c
}
func finish(_ code: Int32) -> Never {
    for child in children { child.stop() }
    if lastPost != nil {
        CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: home, mouseButton: .left)!.post(tap: .cghidEventTap)
    }
    exit(code)
}
func front(over t: CGPoint) -> Child {
    // Below the hovered row and past the window's side: what the popup would cross.
    let c = Child("\(dir)/front", ["\(t.x - 100)", "\(t.y + 24)", "700", "400"], tag: "front")
    wait(1.0); return c
}
func hover(_ title: String, _ path: String, _ args: [String], _ target: String, background: Bool) {
    say("=== \(title)")
    let c = launch(path, args, "app"); wait(0.8)
    let t = c.targets[target]!, f = background ? front(over: t) : nil
    let pids: [Int32: String] = f == nil ? [c.p.processIdentifier: "app"] : [c.p.processIdentifier: "app", f!.p.processIdentifier: "front"]
    say(" before"); order(pids)
    let ok = move(t, c.p.processIdentifier)
    for s in [0.6, 1.0, 1.5, 2.0] { wait(s); say(" after +\(s)"); order(pids) }
    if getenv("POPUP_HOVER_ACT") != nil { wait(2.0); say(" after the act"); order(pids) }
    if ok { CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: home, mouseButton: .left)!.post(tap: .cghidEventTap); lastPost = Date(); wait(0.5) }
    f?.stop(); c.stop(); wait(0.5)
    if !ok { finish(5) }
}
func dismissal(_ b: String) {
    for where_ in ["other", "outside"] {
        say("=== AppKit popover \(b): click \(where_)")
        let c = launch("\(dir)/ref", ["--popover", b], "ref"); wait(1.0)
        if !click(c.targets[where_]!, c.p.processIdentifier) { c.stop(); finish(5) }
        wait(1.5); order([c.p.processIdentifier: "ref"]); c.stop(); wait(0.5)
    }
}
func tracking(_ title: String, _ args: [String], background: Bool) {
    say("=== \(title)")
    let c = launch("\(dir)/ref", ["--tracking"] + args, "ref"); wait(0.8)
    let names = ["always", "activeApp", "keyWindow", "legacyRect", "button"]
    let f = background ? front(over: c.targets["always"]!) : nil
    order(f == nil ? [c.p.processIdentifier: "ref"] : [c.p.processIdentifier: "ref", f!.p.processIdentifier: "front"])
    for n in names {
        say(" hover \(n)")
        if !move(c.targets[n]!, c.p.processIdentifier) { f?.stop(); c.stop(); finish(5) }
        wait(0.6)
    }
    order(f == nil ? [c.p.processIdentifier: "ref"] : [c.p.processIdentifier: "ref", f!.p.processIdentifier: "front"])
    f?.stop(); c.stop(); wait(0.5)
}
// One case: drive <dir> <zgx> case <act> <target>.
if CommandLine.arguments.count > 5, CommandLine.arguments[3] == "case" {
    setenv("POPUP_HOVER_ACT", CommandLine.arguments[4], 1)
    hover("zgx \(CommandLine.arguments[5]), \(CommandLine.arguments[4])", zgx, [], CommandLine.arguments[5], background: false)
    say("DONE")
    finish(0)
}
// What becomes of a popup as its window moves, resizes, miniaturizes, goes full
// screen, closes or leaves its Space: each process acts once, on its own timer.
if CommandLine.arguments.count > 3, CommandLine.arguments[3] == "act" {
    for act in ["move", "resize", "mini", "full"] {
        setenv("POPUP_HOVER_ACT", act, 1)
        hover("AppKit tooltip, \(act)", "\(dir)/ref", ["--tip"], "tip", background: false)
        for (name, target) in [("tooltip", "tip"), ("menu", "row"), ("hover card", "card")] {
            hover("zgx \(name), \(act)", zgx, [], target, background: false)
        }
    }
    setenv("POPUP_HOVER_SECOND", "1", 1)
    for act in ["close", "space"] {
        setenv("POPUP_HOVER_ACT", act, 1)
        for (name, target) in [("tooltip", "tip"), ("menu", "row")] {
            hover("zgx \(name), \(act)", zgx, [], target, background: false)
        }
    }
    say("DONE")
    finish(0)
}
if CommandLine.arguments.count > 3, CommandLine.arguments[3] == "pop2" {
    tracking("AppKit delivery, key window", [], background: false)
    tracking("AppKit delivery, app active, window not key", ["--not-key"], background: false)
    tracking("AppKit delivery, app at the back", [], background: true)
    setenv("POPUP_HOVER_SECOND", "1", 1)
    hover("zgx tooltip, its other window active, over it", zgx, [], "tip", background: false)
    hover("zgx hover card, its other window active, over it", zgx, [], "card", background: false)
    unsetenv("POPUP_HOVER_SECOND")
    for (name, target) in [("menu row, the app's hover state", "row"), ("overlay row, the app's hover state", "overlay"), ("tooltip", "tip"), ("hover card", "card")] {
        hover("zgx \(name), app in front", zgx, [], target, background: false)
        hover("zgx \(name), app at the back", zgx, [], target, background: true)
    }
    say("DONE")
    finish(0)
}
hover("AppKit tooltip, app in front", "\(dir)/ref", ["--tip"], "tip", background: false)
hover("AppKit tooltip, app at the back", "\(dir)/ref", ["--tip"], "tip", background: true)
for b in ["transient", "semitransient", "applicationDefined"] { dismissal(b) }
hover("zgx tooltip, app in front", zgx, [], "tip", background: false)
hover("zgx hover card, app in front", zgx, [], "card", background: false)
hover("zgx tooltip, app at the back", zgx, [], "tip", background: true)
hover("zgx hover card, app at the back", zgx, [], "card", background: true)
say("DONE")
finish(0)
