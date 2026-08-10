// Shape a square image the way macOS expects an app icon to be shaped.
//
// Since Big Sur the convention is a 1024pt canvas with the artwork inside an
// 824pt rounded square — roughly 80% — leaving a transparent margin. The corner
// radius is about 22.37% of that square. Getting the proportions wrong is what
// makes a hand-made icon sit visibly higher or lower than its neighbours in the
// Dock, because macOS does not mask or inset anything for you.
//
//   swift roundicon.swift <input.png> <output.png>

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let args = CommandLine.arguments
guard args.count == 3 else {
    FileHandle.standardError.write("usage: roundicon <in.png> <out.png>\n".data(using: .utf8)!)
    exit(2)
}

let canvas = 1024.0
let inset = 824.0            // Apple's artwork square within the canvas
let radius = inset * 0.2237  // and its corner radius

guard
    let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
    let image = CGImageSourceCreateImageAtIndex(src, 0, nil)
else {
    FileHandle.standardError.write("could not read input\n".data(using: .utf8)!)
    exit(1)
}

guard
    let context = CGContext(
        data: nil,
        width: Int(canvas),
        height: Int(canvas),
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )
else {
    FileHandle.standardError.write("could not create context\n".data(using: .utf8)!)
    exit(1)
}

context.interpolationQuality = .high

let origin = (canvas - inset) / 2
let rect = CGRect(x: origin, y: origin, width: inset, height: inset)
context.addPath(CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil))
context.clip()

// The source is square, so filling the rounded square needs no cropping — but
// drawing into the rect rather than the canvas is what keeps the margin.
context.draw(image, in: rect)

guard
    let out = context.makeImage(),
    let dest = CGImageDestinationCreateWithURL(
        URL(fileURLWithPath: args[2]) as CFURL, UTType.png.identifier as CFString, 1, nil)
else {
    FileHandle.standardError.write("could not write output\n".data(using: .utf8)!)
    exit(1)
}

CGImageDestinationAddImage(dest, out, nil)
CGImageDestinationFinalize(dest)
print("wrote \(args[2])")
