// Generates the vision fixture used by the live tests: a 400x120 px PNG with "Hello world".
// Usage: swift scripts/make-vision-fixture.swift <out.png>
// macOS only. Uses an explicit pixel size (not lockFocus, which doubles on Retina).
import AppKit
import Foundation

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: swift make-vision-fixture.swift <out.png>\n".utf8))
    exit(2)
}
let outPath = CommandLine.arguments[1]

let width = 400
let height = 120
guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: width,
    pixelsHigh: height,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
), let context = NSGraphicsContext(bitmapImageRep: rep) else {
    FileHandle.standardError.write(Data("cannot create bitmap context\n".utf8))
    exit(1)
}
rep.size = NSSize(width: width, height: height)

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = context

NSColor.white.setFill()
NSRect(x: 0, y: 0, width: width, height: height).fill()

let attributes: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 48),
    .foregroundColor: NSColor(calibratedWhite: 0.05, alpha: 1.0),
]
let text = NSAttributedString(string: "Hello world", attributes: attributes)
let size = text.size()
text.draw(at: NSPoint(x: (CGFloat(width) - size.width) / 2, y: (CGFloat(height) - size.height) / 2))

NSGraphicsContext.restoreGraphicsState()

guard let png = rep.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write(Data("PNG encoding failed\n".utf8))
    exit(1)
}
do {
    try png.write(to: URL(fileURLWithPath: outPath))
} catch {
    FileHandle.standardError.write(Data("cannot write \(outPath): \(error)\n".utf8))
    exit(1)
}
