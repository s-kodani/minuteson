// Regenerate on macOS: swift tools/dmg/render-background.swift tools/dmg/background.tiff
// The TIFF has 1x and 2x representations at the same 660 x 440 point size.
import AppKit

let width = 660.0
let height = 440.0
var representations: [NSBitmapImageRep] = []

for scale in [1.0, 2.0] {
    let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(width * scale), pixelsHigh: Int(height * scale),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    let context = NSGraphicsContext(bitmapImageRep: bitmap)!
    NSGraphicsContext.saveGraphicsState()
    context.cgContext.translateBy(x: 0, y: height * scale)
    context.cgContext.scaleBy(x: scale, y: -scale)
    NSGraphicsContext.current = NSGraphicsContext(cgContext: context.cgContext, flipped: true)

    NSColor(srgbRed: 0.965, green: 0.969, blue: 0.976, alpha: 1).setFill()
    NSBezierPath(rect: NSRect(x: 0, y: 0, width: width, height: height)).fill()

    func text(_ value: String, y: Double, size: Double, weight: NSFont.Weight = .regular,
              color: NSColor = NSColor(srgbRed: 0.18, green: 0.20, blue: 0.24, alpha: 1)) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        (value as NSString).draw(in: NSRect(x: 24, y: y, width: width - 48, height: 44), withAttributes: [
            .font: NSFont.systemFont(ofSize: size, weight: weight),
            .foregroundColor: color, .paragraphStyle: paragraph,
        ])
    }
    text("MinutesOn をインストール", y: 43, size: 27, weight: .semibold)
    text("左の MinutesOn を右の Applications にドラッグしてください。", y: 99, size: 16)

    // Aligned with the Finder icon centres in mon_macos_dmg.py.
    NSColor(srgbRed: 0.25, green: 0.43, blue: 0.78, alpha: 1).setStroke()
    let arrow = NSBezierPath()
    arrow.lineWidth = 5
    arrow.lineCapStyle = .round
    arrow.lineJoinStyle = .round
    arrow.move(to: NSPoint(x: 292, y: 220))
    arrow.line(to: NSPoint(x: 368, y: 220))
    arrow.move(to: NSPoint(x: 350, y: 202))
    arrow.line(to: NSPoint(x: 368, y: 220))
    arrow.line(to: NSPoint(x: 350, y: 238))
    arrow.stroke()

    // Finder's title bar takes 32 points from the saved 440-point window.
    // In the visible 408-point content area, match the top and bottom margins.
    text("コピーが完了したら、Applications から MinutesOn を起動します。", y: 317, size: 14)
    text("その後、このディスクを取り出してください。", y: 346, size: 13, color: .darkGray)
    NSGraphicsContext.restoreGraphicsState()
    bitmap.size = NSSize(width: width, height: height)
    representations.append(bitmap)
}

let data = NSBitmapImageRep.representationOfImageReps(
    in: representations, using: .tiff,
    properties: [.compressionMethod: NSBitmapImageRep.TIFFCompression.lzw.rawValue])!
try data.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
