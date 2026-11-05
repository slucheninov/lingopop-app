// macOS generator. Run from the repository root. The Windows status icons are exactly 32x32 RGBA; the macOS template is 18 pt high at 3x.
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

// Menu-bar template: the frozen Swift app's idle image (`StatusItemController.makeIdleImage`), the text
// "AЇ" in the 13 pt semibold system font, black on transparent so AppKit tints it. Drawn at 3x on an
// 18 pt high canvas (tray-icon scales every macOS tray image to 18 pt high, width in proportion).
let scale: CGFloat = 3
let glyph = NSAttributedString(string: "AЇ", attributes: [.font: NSFont.systemFont(ofSize: 13 * scale, weight: .semibold), .foregroundColor: NSColor.black])
let glyphSize = glyph.size()
let templateHeight = Int(18 * scale)
let templateWidth = Int(ceil(glyphSize.width)) + 2
let template = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: templateWidth, pixelsHigh: templateHeight, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: template)
glyph.draw(at: NSPoint(x: 1, y: (CGFloat(templateHeight) - glyphSize.height) / 2))
NSGraphicsContext.restoreGraphicsState()
try template.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(directory)/mac-template.png"))
