import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// What the screens may do with a file someone sent. Its name, its type and its bytes are theirs to
/// choose: the type comes from the bytes, and an image is measured before anything decodes it.
public enum ReceivedFile {
    /// The image formats shown in a conversation, told apart by their first bytes.
    public enum ImageKind: Equatable, Sendable {
        case png, jpeg, gif, webp, heic

        /// What ImageIO must also find in the file.
        var types: [UTType] {
            switch self {
            case .png: [.png]
            case .jpeg: [.jpeg]
            case .gif: [.gif]
            case .webp: [.webP]
            case .heic: [.heic, .heif]
            }
        }
    }

    /// Larger images are not decoded (30000×30000 pixels would take 3.6 GB): a phone photo is 48 MP.
    public static let maxPixels = 50_000_000
    public static let maxSide = 12_000
    /// The longest side, in pixels, of an image drawn in a conversation.
    public static let thumbnailSide = 1024

    /// Types Quick Look shows as a web page, which can load remote content.
    static let webContent: [UTType] = [.html, .xml, .svg, .webArchive]

    /// What a saved file shows. Sendable: a CGImage never changes once made.
    public struct Look: @unchecked Sendable {
        /// The image to draw, for one of the formats above that is not too large.
        public let thumbnail: CGImage?
        /// Quick Look may open it.
        public let previewable: Bool
    }

    public static func imageKind(_ head: Data) -> ImageKind? {
        let b = [UInt8](head.prefix(12))
        func starts(_ p: [UInt8], at i: Int = 0) -> Bool { b.count >= i + p.count && Array(b[i..<i + p.count]) == p }
        if starts([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) { return .png }
        if starts([0xFF, 0xD8, 0xFF]) { return .jpeg }
        if starts(Array("GIF87a".utf8)) || starts(Array("GIF89a".utf8)) { return .gif }
        if starts(Array("RIFF".utf8)) && starts(Array("WEBP".utf8), at: 8) { return .webp }
        if starts(Array("ftyp".utf8), at: 4), b.count >= 12,
           ["heic", "heix", "hevc", "hevx", "mif1", "msf1"].contains(String(decoding: b[8..<12], as: UTF8.self)) {
            return .heic
        }
        return nil
    }

    public static func imageKind(at url: URL) -> ImageKind? {
        guard let file = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? file.close() }
        return (try? file.read(upToCount: 12)).flatMap(imageKind)
    }

    public static func fits(width: Int, height: Int) -> Bool {
        width > 0 && height > 0 && width <= maxSide && height <= maxSide && width * height <= maxPixels
    }

    /// A small rendering of a received image; nil for any other file, or one too large to decode.
    /// Blocking: call it off the main thread.
    public static func thumbnail(at url: URL, maxPixelSize: Int = thumbnailSide) -> CGImage? {
        guard let kind = imageKind(at: url),
              let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let found = CGImageSourceGetType(source).flatMap({ UTType($0 as String) }),
              kind.types.contains(found),
              let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = props[kCGImagePropertyPixelWidth] as? Int,
              let height = props[kCGImagePropertyPixelHeight] as? Int,
              fits(width: width, height: height)
        else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    /// What to show for a saved file. Quick Look goes by the name's extension: it never opens a web
    /// page (HTML, SVG, XML), and opens an image only when it is one we could draw. Blocking.
    public static func look(at url: URL, mime: String) -> Look {
        let thumbnail = thumbnail(at: url)
        let types = [UTType(filenameExtension: url.pathExtension), UTType(mimeType: mime)].compactMap { $0 }
        let web = types.contains { t in webContent.contains { t.conforms(to: $0) } }
        let image = types.contains { $0.conforms(to: .image) } || imageKind(at: url) != nil
        return Look(thumbnail: thumbnail, previewable: !web && (!image || thumbnail != nil))
    }
}
