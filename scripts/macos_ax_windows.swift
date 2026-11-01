// What assistive apps (System Events, Raycast) see of a process's windows,
// sampled over time: AXWindows' count, and per window whether AXSize and
// AXPosition are settable; beside the window server's on-screen windows of
// the process (CGWindowList), and whether the process is frontmost.
// Never prompts: without accessibility trust it says so and exits 2.
// swiftc -O -o macos_ax_windows macos_ax_windows.swift
// macos_ax_windows <pid> [seconds] [interval ms]: one line per change.
import AppKit
import ApplicationServices

let args = CommandLine.arguments
guard args.count >= 2, let pid = pid_t(args[1]) else {
    print("usage: macos_ax_windows <pid> [seconds] [interval ms]"); exit(64)
}
let seconds = args.count > 2 ? Double(args[2])! : 5
let interval = args.count > 3 ? Double(args[3])! / 1000 : 0.1
if !AXIsProcessTrusted() {
    print("UNTRUSTED: this process has no accessibility permission (not requested)")
    exit(2)
}
let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 1.0)
func windows() -> (Int, [String])? {
    var value: CFTypeRef?
    let err = AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value)
    guard err == .success, let list = value as? [AXUIElement] else {
        return err == .success ? (0, []) : nil
    }
    let described = list.map { w -> String in
        var size = DarwinBoolean(false), pos = DarwinBoolean(false)
        AXUIElementIsAttributeSettable(w, kAXSizeAttribute as CFString, &size)
        AXUIElementIsAttributeSettable(w, kAXPositionAttribute as CFString, &pos)
        var title: CFTypeRef?
        AXUIElementCopyAttributeValue(w, kAXTitleAttribute as CFString, &title)
        return "\((title as? String) ?? "?") size:\(size.boolValue) pos:\(pos.boolValue)"
    }
    return (list.count, described)
}
func onScreen() -> Int {
    let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    return info.filter { ($0[kCGWindowOwnerPID as String] as? pid_t) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }.count
}
let start = Date()
var last = ""
while Date().timeIntervalSince(start) < seconds {
    let front = NSWorkspace.shared.frontmostApplication?.processIdentifier == pid
    let line: String
    if let (count, described) = windows() {
        line = "ax=\(count) screen=\(onScreen()) front=\(front) \(described.joined(separator: "; "))"
    } else {
        line = "ax=ERR screen=\(onScreen()) front=\(front)"
    }
    if line != last {
        print(String(format: "%7.0f ms ", Date().timeIntervalSince(start) * 1000) + line)
        fflush(stdout)
        last = line
    }
    Thread.sleep(forTimeInterval: interval)
}
print("END")
