// 导出 AppKit 语义色在浅色（aqua）与深色（darkAqua）外观下的 sRGB 值。
// 用法：swift tools/colors-dump.swift [输出文件，默认 reference/colors.json]
import AppKit

let colors: [(String, NSColor)] = [
    ("labelColor", .labelColor),
    ("secondaryLabelColor", .secondaryLabelColor),
    ("tertiaryLabelColor", .tertiaryLabelColor),
    ("quaternaryLabelColor", .quaternaryLabelColor),
    ("textColor", .textColor),
    ("placeholderTextColor", .placeholderTextColor),
    ("selectedTextColor", .selectedTextColor),
    ("textBackgroundColor", .textBackgroundColor),
    ("selectedTextBackgroundColor", .selectedTextBackgroundColor),
    ("keyboardFocusIndicatorColor", .keyboardFocusIndicatorColor),
    ("unemphasizedSelectedTextColor", .unemphasizedSelectedTextColor),
    ("unemphasizedSelectedTextBackgroundColor", .unemphasizedSelectedTextBackgroundColor),
    ("linkColor", .linkColor),
    ("separatorColor", .separatorColor),
    ("selectedContentBackgroundColor", .selectedContentBackgroundColor),
    ("unemphasizedSelectedContentBackgroundColor", .unemphasizedSelectedContentBackgroundColor),
    ("selectedMenuItemTextColor", .selectedMenuItemTextColor),
    ("gridColor", .gridColor),
    ("headerTextColor", .headerTextColor),
    ("alternatingContentBackground0", NSColor.alternatingContentBackgroundColors[0]),
    ("alternatingContentBackground1", NSColor.alternatingContentBackgroundColors[1]),
    ("controlAccentColor", .controlAccentColor),
    ("controlColor", .controlColor),
    ("controlBackgroundColor", .controlBackgroundColor),
    ("controlTextColor", .controlTextColor),
    ("disabledControlTextColor", .disabledControlTextColor),
    ("selectedControlColor", .selectedControlColor),
    ("selectedControlTextColor", .selectedControlTextColor),
    ("alternateSelectedControlTextColor", .alternateSelectedControlTextColor),
    ("windowBackgroundColor", .windowBackgroundColor),
    ("windowFrameTextColor", .windowFrameTextColor),
    ("underPageBackgroundColor", .underPageBackgroundColor),
    ("findHighlightColor", .findHighlightColor),
    ("highlightColor", .highlightColor),
    ("shadowColor", .shadowColor),
    ("systemRed", .systemRed),
    ("systemOrange", .systemOrange),
    ("systemYellow", .systemYellow),
    ("systemGreen", .systemGreen),
    ("systemMint", .systemMint),
    ("systemTeal", .systemTeal),
    ("systemCyan", .systemCyan),
    ("systemBlue", .systemBlue),
    ("systemIndigo", .systemIndigo),
    ("systemPurple", .systemPurple),
    ("systemPink", .systemPink),
    ("systemBrown", .systemBrown),
    ("systemGray", .systemGray),
    ("systemFill", .systemFill),
    ("secondarySystemFill", .secondarySystemFill),
    ("tertiarySystemFill", .tertiarySystemFill),
    ("quaternarySystemFill", .quaternarySystemFill),
]

func hex(_ color: NSColor) -> String {
    guard let c = color.usingColorSpace(.sRGB) else { return "unresolved" }
    let r = Int((c.redComponent * 255).rounded())
    let g = Int((c.greenComponent * 255).rounded())
    let b = Int((c.blueComponent * 255).rounded())
    let a = Int((c.alphaComponent * 255).rounded())
    return String(format: "#%02x%02x%02x%02x", r, g, b, a)
}

var result: [String: [String: String]] = [:]
for (mode, name) in [("light", NSAppearance.Name.aqua), ("dark", NSAppearance.Name.darkAqua)] {
    let appearance = NSAppearance(named: name)!
    var table: [String: String] = [:]
    appearance.performAsCurrentDrawingAppearance {
        for (key, color) in colors {
            table[key] = hex(color)
        }
    }
    result[mode] = table
}

let out = CommandLine.arguments.dropFirst().first ?? "reference/colors.json"
let data = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
try FileManager.default.createDirectory(
    atPath: (out as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
try data.write(to: URL(fileURLWithPath: out))
print("写入 \(colors.count) 个语义色（浅色+深色）到 \(out)")
