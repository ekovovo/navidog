import AppKit

// Rasterize the app SVG into a macOS iconset. AppKit keeps the transparent
// margin; Quick Look thumbnails do not.
let args = CommandLine.arguments
guard args.count == 3, let image = NSImage(contentsOf: URL(fileURLWithPath: args[1])) else {
    fputs("usage: raster-icon.swift <svg> <iconset-dir>\n", stderr)
    exit(1)
}
let dir = URL(fileURLWithPath: args[2], isDirectory: true)
try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

let sizes = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]

for (name, side) in sizes {
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: side,
        pixelsHigh: side,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ) else {
        fputs("bitmap failed\n", stderr)
        exit(1)
    }
    let box = NSRect(x: 0, y: 0, width: side, height: side)
    rep.size = box.size
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSColor.clear.set()
    box.fill()
    image.draw(in: box)
    NSGraphicsContext.restoreGraphicsState()
    guard let png = rep.representation(using: .png, properties: [:]) else {
        fputs("png failed\n", stderr)
        exit(1)
    }
    try png.write(to: dir.appendingPathComponent(name))
}
