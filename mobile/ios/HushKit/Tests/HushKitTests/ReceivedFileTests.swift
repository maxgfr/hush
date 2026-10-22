import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import HushKit

struct ReceivedFileTests {
    let dir: URL

    init() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("hush-files-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    private func write(_ data: Data, _ name: String) throws -> URL {
        let url = dir.appendingPathComponent(name)
        try data.write(to: url)
        return url
    }

    /// A real PNG, drawn and encoded here.
    private func png(width: Int, height: Int) throws -> Data {
        let ctx = try #require(CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ))
        ctx.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let image = try #require(ctx.makeImage())
        let out = NSMutableData()
        let dest = try #require(CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(dest, image, nil)
        #expect(CGImageDestinationFinalize(dest))
        return out as Data
    }

    /// A decode bomb: a black 1-bit PNG, 30000 pixels a side, in about 110 KB.
    private func pngBomb(side: Int) throws -> Data {
        func chunk(_ type: String, _ body: [UInt8]) -> [UInt8] {
            let tagged = Array(type.utf8) + body
            return be(UInt32(body.count)) + tagged + be(crc32(tagged))
        }
        let raw = side * (1 + (side + 7) / 8)
        let deflated = try (Data(count: raw) as NSData).compressed(using: .zlib) as Data
        // zlib's header, then the Adler-32 of `raw` zero bytes.
        let idat = [0x78, 0x9C] + [UInt8](deflated) + be(UInt32(raw % 65521) << 16 | 1)
        let ihdr = be(UInt32(side)) + be(UInt32(side)) + [1, 0, 0, 0, 0]
        return Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] + chunk("IHDR", ihdr) + chunk("IDAT", idat) + chunk("IEND", []))
    }

    @Test func formatsComeFromTheBytes() {
        #expect(ReceivedFile.imageKind(Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0])) == .png)
        #expect(ReceivedFile.imageKind(Data([0xFF, 0xD8, 0xFF, 0xE0])) == .jpeg)
        #expect(ReceivedFile.imageKind(Data("GIF89a…".utf8)) == .gif)
        #expect(ReceivedFile.imageKind(Data("RIFF\0\0\0\0WEBPVP8 ".utf8)) == .webp)
        #expect(ReceivedFile.imageKind(Data([0, 0, 0, 0x18] + Array("ftypheic".utf8))) == .heic)
        #expect(ReceivedFile.imageKind(Data("<svg xmlns=".utf8)) == nil)
        #expect(ReceivedFile.imageKind(Data("%PDF-1.7".utf8)) == nil)
        #expect(ReceivedFile.imageKind(Data()) == nil)
    }

    @Test func sizesAreBounded() {
        #expect(ReceivedFile.fits(width: 8064, height: 6048))
        #expect(!ReceivedFile.fits(width: 30000, height: 30000))
        #expect(!ReceivedFile.fits(width: 12001, height: 10))
        #expect(!ReceivedFile.fits(width: 10000, height: 10000))
        #expect(!ReceivedFile.fits(width: 0, height: 10))
    }

    @Test func anImageIsDrawnSmall() throws {
        let url = try write(png(width: 2048, height: 1024), "photo.png")
        let thumb = try #require(ReceivedFile.thumbnail(at: url))
        #expect(thumb.width == 1024 && thumb.height == 512)
        let look = ReceivedFile.look(at: url, mime: "image/png")
        #expect(look.thumbnail != nil && look.previewable)
    }

    @Test func aHugeImageIsNeverDecoded() throws {
        let bomb = try pngBomb(side: 30000)
        #expect(bomb.count < 200_000)
        let url = try write(bomb, "bomb.png")
        let source = try #require(CGImageSourceCreateWithURL(url as CFURL, nil))
        let props = try #require(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        #expect(props[kCGImagePropertyPixelWidth] as? Int == 30000)
        #expect(ReceivedFile.thumbnail(at: url) == nil)
        // Not even in Quick Look.
        let look = ReceivedFile.look(at: url, mime: "image/png")
        #expect(look.thumbnail == nil && !look.previewable)
    }

    @Test func theDeclaredTypeIsNotTrusted() throws {
        let svg = Data(#"<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.org/x"/></svg>"#.utf8)
        // Called an image: neither drawn nor opened.
        let disguised = ReceivedFile.look(at: try write(svg, "cat.png"), mime: "image/png")
        #expect(disguised.thumbnail == nil && !disguised.previewable)
        // Web pages never open in Quick Look, whatever the MIME says.
        #expect(!ReceivedFile.look(at: try write(svg, "cat.svg"), mime: "text/plain").previewable)
        #expect(!ReceivedFile.look(at: try write(Data("<html>".utf8), "page.html"), mime: "application/pdf").previewable)
        #expect(!ReceivedFile.look(at: try write(Data("<html>".utf8), "page.txt"), mime: "text/html").previewable)
        // A real image declared as something else is still drawn.
        let real = ReceivedFile.look(at: try write(png(width: 10, height: 10), "data.bin"), mime: "application/octet-stream")
        #expect(real.thumbnail != nil && real.previewable)
        // Other documents open.
        #expect(ReceivedFile.look(at: try write(Data("%PDF-1.7".utf8), "a.pdf"), mime: "application/pdf").previewable)
        #expect(ReceivedFile.look(at: try write(Data("hello".utf8), "a.txt"), mime: "text/plain").previewable)
    }
}

private func be(_ v: UInt32) -> [UInt8] {
    [UInt8(v >> 24), UInt8(v >> 16 & 0xFF), UInt8(v >> 8 & 0xFF), UInt8(v & 0xFF)]
}

private func crc32(_ bytes: [UInt8]) -> UInt32 {
    var crc: UInt32 = 0xFFFF_FFFF
    for b in bytes {
        crc ^= UInt32(b)
        for _ in 0..<8 { crc = crc & 1 == 1 ? (crc >> 1) ^ 0xEDB8_8320 : crc >> 1 }
    }
    return ~crc
}
