// A running app's sliders as VoiceOver reads them and moves them: for the
// slider named <name>, its value, range and actions as JSON, then its value
// after AXIncrement and after AXDecrement. Acts only on that app's slider.
//
//     swiftc -O scripts/macos_ax_slider.swift -o /tmp/macos_ax_slider
//     /tmp/macos_ax_slider <pid> <name>
//
// Exits 3 when this process may not read other apps' accessibility
// (System Settings, Privacy & Security, Accessibility), 4 with no such slider.
import ApplicationServices
import Foundation

let arguments = CommandLine.arguments
guard arguments.count == 3, let pid = pid_t(arguments[1]) else {
    FileHandle.standardError.write("usage: macos_ax_slider <pid> <name>\n".data(using: .utf8)!)
    exit(2)
}
let name = arguments[2]
guard AXIsProcessTrusted() else {
    FileHandle.standardError.write("not trusted for accessibility\n".data(using: .utf8)!)
    exit(3)
}
let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 5)

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}
func string(_ value: AnyObject?) -> String? {
    if let text = value as? String { return text }
    if let number = value as? NSNumber { return number.stringValue }
    return nil
}
func find(_ element: AXUIElement, _ depth: Int = 0) -> AXUIElement? {
    if string(attribute(element, kAXRoleAttribute)) == kAXSliderRole,
        [kAXTitleAttribute, kAXDescriptionAttribute].contains(where: { string(attribute(element, $0)) == name })
    {
        return element
    }
    guard depth < 40, let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] else {
        return nil
    }
    for child in children {
        if let found = find(child, depth + 1) { return found }
    }
    return nil
}
var slider: AXUIElement?
for _ in 0..<50 {
    slider = find(app)
    if slider != nil { break }
    Thread.sleep(forTimeInterval: 0.2)
}
guard let slider else {
    FileHandle.standardError.write("no slider named \(name)\n".data(using: .utf8)!)
    exit(4)
}
func value() -> String { string(attribute(slider, kAXValueAttribute)) ?? "" }
func settled(after action: String) -> String {
    let before = value()
    AXUIElementPerformAction(slider, action as CFString)
    // Up to 5 s: AXValue trails the app's tree update.
    for _ in 0..<50 where value() == before {
        Thread.sleep(forTimeInterval: 0.1)
    }
    return value()
}
var actions: CFArray?
AXUIElementCopyActionNames(slider, &actions)
var out: [String: Any] = [
    "value": value(),
    "min": string(attribute(slider, kAXMinValueAttribute)) ?? "",
    "max": string(attribute(slider, kAXMaxValueAttribute)) ?? "",
    "actions": (actions as? [String]) ?? [],
]
out["incremented"] = settled(after: kAXIncrementAction)
out["decremented"] = settled(after: kAXDecrementAction)
let data = try! JSONSerialization.data(withJSONObject: out, options: [.sortedKeys])
FileHandle.standardOutput.write(data)
