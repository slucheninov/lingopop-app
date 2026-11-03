// macOS generator. Run from the repository root; output is always exactly 32x32 RGBA.
import AppKit
let directory = CommandLine.arguments[1]
try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
let source = NSImage(contentsOfFile: "docs/icon.png")!
for (index, name) in ["idle", "working-1", "working-2", "error"].enumerated() {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 32, pixelsHigh: 32, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    source.draw(in: NSRect(x: 0, y: 0, width: 32, height: 32))
    if index == 1 || index == 2 {
        NSColor.systemBlue.setStroke()
        let arc = NSBezierPath()
        arc.lineWidth = 3
        arc.appendArc(withCenter: NSPoint(x: 16, y: 16), radius: 14, startAngle: index == 1 ? 0 : 180, endAngle: index == 1 ? 150 : 330)
        arc.stroke()
    } else if index == 3 {
        NSColor.systemRed.setFill()
        NSBezierPath(ovalIn: NSRect(x: 22, y: 0, width: 10, height: 10)).fill()
    }
    NSGraphicsContext.restoreGraphicsState()
    try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(directory)/\(name).png"))
}
