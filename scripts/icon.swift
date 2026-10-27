// Draws gitroute's app icon, a 1024 px PNG, to the path given: two transit
// lines leaving one station on a dark tile. Run through scripts/icon.sh.
import AppKit

let size = 1024.0
let out = CommandLine.arguments[1]
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(
    data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
    space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
// Top-left origin, like the design grid.
ctx.translateBy(x: 0, y: size)
ctx.scaleBy(x: 1, y: -1)

func rgb(_ r: Double, _ g: Double, _ b: Double, _ a: Double = 1) -> CGColor {
    CGColor(colorSpace: space, components: [r / 255, g / 255, b / 255, a])!
}

// The macOS icon grid: an 824 pt tile centered on the canvas.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let shape = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: 10), blur: 24, color: rgb(0, 0, 0, 0.3))
ctx.addPath(shape)
ctx.setFillColor(rgb(28, 29, 33))
ctx.fillPath()
ctx.restoreGState()

ctx.saveGState()
ctx.addPath(shape)
ctx.clip()
let ground = CGGradient(
    colorsSpace: space, colors: [rgb(52, 54, 62), rgb(22, 23, 27)] as CFArray,
    locations: [0, 1])!
ctx.drawLinearGradient(
    ground, start: CGPoint(x: 512, y: 100), end: CGPoint(x: 512, y: 924), options: [])
ctx.restoreGState()

let green = rgb(0, 170, 75)
let blue = rgb(70, 130, 255)
let paper = rgb(246, 246, 248)
let width = 70.0

func line(_ points: [CGPoint], _ color: CGColor) {
    ctx.beginPath()
    ctx.move(to: points[0])
    for p in points.dropFirst() { ctx.addLine(to: p) }
    ctx.setStrokeColor(color)
    ctx.setLineWidth(width)
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.strokePath()
}

func station(_ c: CGPoint, ring: CGColor?, radius: Double = 58) {
    let r = CGRect(x: c.x - radius, y: c.y - radius, width: radius * 2, height: radius * 2)
    ctx.setFillColor(paper)
    ctx.fillEllipse(in: r)
    if let ring {
        ctx.setStrokeColor(ring)
        ctx.setLineWidth(20)
        ctx.strokeEllipse(in: r.insetBy(dx: 10, dy: 10))
    }
}

// One repo's line splits: the default line runs straight on, the other
// account's line branches off at 45° and runs parallel to it.
let (x0, x1) = (430.0, 600.0)
let (top, fork, bottom) = (254.0, 612.0, 798.0)
line([CGPoint(x: x0, y: fork), CGPoint(x: x1, y: fork - (x1 - x0)), CGPoint(x: x1, y: top)], blue)
line([CGPoint(x: x0, y: bottom), CGPoint(x: x0, y: top)], green)
station(CGPoint(x: x0, y: top), ring: green)
station(CGPoint(x: x1, y: top), ring: blue)
station(CGPoint(x: x0, y: fork), ring: nil, radius: 30)
station(CGPoint(x: x0, y: bottom), ring: green)

let image = ctx.makeImage()!
let dest = CGImageDestinationCreateWithURL(
    URL(fileURLWithPath: out) as CFURL, "public.png" as CFString, 1, nil)!
CGImageDestinationAddImage(dest, image, nil)
CGImageDestinationFinalize(dest)
