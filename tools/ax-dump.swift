// 通过辅助功能 API 导出运行中 Navicat 各窗口的元素几何（相对窗口左上角，单位 pt）。
// 需要给运行它的终端授予“辅助功能”权限。
// 用法：swift tools/ax-dump.swift [bundle id，默认 com.navicat.NavicatPremium] [输出目录，默认 reference/ax]
import AppKit
import ApplicationServices

let args = Array(CommandLine.arguments.dropFirst())
let bundleId = args.first ?? "com.navicat.NavicatPremium"
let outDir = args.count > 1 ? args[1] : "reference/ax"

let promptKey = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
guard AXIsProcessTrustedWithOptions([promptKey: true] as CFDictionary) else {
    FileHandle.standardError.write("未获得辅助功能权限：系统设置 > 隐私与安全性 > 辅助功能，勾选当前终端后重试\n".data(using: .utf8)!)
    exit(2)
}
guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).first else {
    FileHandle.standardError.write("\(bundleId) 未运行\n".data(using: .utf8)!)
    exit(3)
}

func attr(_ el: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    return AXUIElementCopyAttributeValue(el, name as CFString, &value) == .success ? value : nil
}

func point(_ el: AXUIElement) -> CGPoint? {
    guard let v = attr(el, kAXPositionAttribute) else { return nil }
    var p = CGPoint.zero
    return AXValueGetValue(v as! AXValue, .cgPoint, &p) ? p : nil
}

func size(_ el: AXUIElement) -> CGSize? {
    guard let v = attr(el, kAXSizeAttribute) else { return nil }
    var s = CGSize.zero
    return AXValueGetValue(v as! AXValue, .cgSize, &s) ? s : nil
}

func text(_ el: AXUIElement, _ name: String) -> String? {
    guard let s = attr(el, name) as? String, !s.isEmpty else { return nil }
    return String(s.prefix(80))
}

// 树很深（结果网格每个单元格都是元素），限制深度与子节点数量避免导出过大。
let maxDepth = 14
let maxChildren = 60

func dump(_ el: AXUIElement, origin: CGPoint, depth: Int) -> [String: Any] {
    var node: [String: Any] = [:]
    node["role"] = text(el, kAXRoleAttribute) ?? "?"
    if let v = text(el, kAXSubroleAttribute) { node["subrole"] = v }
    if let v = text(el, kAXTitleAttribute) { node["title"] = v }
    if let v = text(el, kAXDescriptionAttribute) { node["desc"] = v }
    if let v = text(el, kAXIdentifierAttribute) { node["identifier"] = v }
    if let v = attr(el, kAXValueAttribute) as? String, !v.isEmpty { node["value"] = String(v.prefix(80)) }
    if let p = point(el), let s = size(el) {
        node["frame"] = [p.x - origin.x, p.y - origin.y, s.width, s.height]
    }
    if depth < maxDepth, let children = attr(el, kAXChildrenAttribute) as? [AXUIElement] {
        let list = children.prefix(maxChildren).map { dump($0, origin: origin, depth: depth + 1) }
        if !list.isEmpty { node["children"] = list }
        if children.count > maxChildren { node["truncatedChildren"] = children.count - maxChildren }
    }
    return node
}

let axApp = AXUIElementCreateApplication(app.processIdentifier)
let windows = attr(axApp, kAXWindowsAttribute) as? [AXUIElement] ?? []
try FileManager.default.createDirectory(atPath: outDir, withIntermediateDirectories: true)
let stamp = Int(Date().timeIntervalSince1970)
for (i, window) in windows.enumerated() {
    let origin = point(window) ?? .zero
    let tree = dump(window, origin: origin, depth: 0)
    let title = (text(window, kAXTitleAttribute) ?? "window\(i)")
        .replacingOccurrences(of: "/", with: "_")
    let path = "\(outDir)/\(stamp)-\(i)-\(title).json"
    let data = try JSONSerialization.data(withJSONObject: tree, options: [.prettyPrinted, .sortedKeys])
    try data.write(to: URL(fileURLWithPath: path))
    print("写入 \(path)")
}
if windows.isEmpty { print("\(bundleId) 没有打开的窗口") }

// 菜单栏大部分由代码动态生成，nib 里只有骨架，运行时导出才完整。
if let menuBar = attr(axApp, kAXMenuBarAttribute) {
    let tree = dump(menuBar as! AXUIElement, origin: .zero, depth: 0)
    let path = "\(outDir)/\(stamp)-menubar.json"
    let data = try JSONSerialization.data(withJSONObject: tree, options: [.prettyPrinted, .sortedKeys])
    try data.write(to: URL(fileURLWithPath: path))
    print("写入 \(path)")
}
