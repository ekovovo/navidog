// 截取指定进程的主窗口（用于开发时目视检查，不做像素对比）。
// 需要给终端授予“屏幕录制”权限。
// 用法：swift tools/window-shot.swift <进程名> <输出 png>
import AppKit
import CoreGraphics

let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 2 else {
    print("用法：swift tools/window-shot.swift <进程名> <输出 png>")
    exit(1)
}
let owner = args[0]
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let windows = list.filter { ($0[kCGWindowOwnerName as String] as? String) == owner && ($0[kCGWindowLayer as String] as? Int) == 0 }
guard let win = windows.max(by: {
    let a = $0[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
    let b = $1[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
    return (a["Width"] ?? 0) * (a["Height"] ?? 0) < (b["Width"] ?? 0) * (b["Height"] ?? 0)
}), let id = win[kCGWindowNumber as String] as? Int else {
    print("没有找到 \(owner) 的窗口")
    exit(2)
}
let task = Process()
task.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
task.arguments = ["-x", "-o", "-l", String(id), args[1]]
try task.run()
task.waitUntilExit()
print("已保存 \(args[1])（窗口 \(id)）")
