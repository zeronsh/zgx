// Reversible display-mode selection for real 60/120 Hz scroll conditions.
import CoreGraphics
import Foundation

let display = CGMainDisplayID()
guard let current = CGDisplayCopyDisplayMode(display) else {
    fatalError("no display mode")
}
let available = (CGDisplayCopyAllDisplayModes(display, [kCGDisplayShowDuplicateLowResolutionModes: true] as CFDictionary) as? [CGDisplayMode]) ?? []
let modes = [current] + available.filter { $0.ioDisplayModeID != current.ioDisplayModeID }
func describe(_ mode: CGDisplayMode) -> [String: Any] {
    ["id": mode.ioDisplayModeID, "width": mode.width, "height": mode.height,
     "pixel_width": mode.pixelWidth, "pixel_height": mode.pixelHeight,
     "hz": mode.refreshRate]
}
func output(_ value: [String: Any]) {
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    print(String(data: data, encoding: .utf8)!)
}
let args = Array(CommandLine.arguments.dropFirst())
if args.isEmpty || args[0] == "status" {
    output(["display": display, "current": describe(current), "modes": modes.map(describe)])
} else {
    guard args.count == 2, args[0] == "set", let id = UInt32(args[1]),
          let mode = modes.first(where: { $0.ioDisplayModeID == id }) else {
        fputs("usage: scroll-display status | set MODE_ID\n", stderr)
        exit(2)
    }
    let result = CGDisplaySetDisplayMode(display, mode, nil)
    guard result == .success else {
        fputs("display mode change failed: \(result.rawValue)\n", stderr)
        exit(1)
    }
    output(["display": display, "set": describe(mode)])
}
