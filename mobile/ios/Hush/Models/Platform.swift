import SwiftUI
#if canImport(UIKit)
import UIKit
import UniformTypeIdentifiers
#endif

/// The few UIKit pieces, kept here so the rest of the app is plain SwiftUI.
enum Clipboard {
    /// How long a copied link or identifier stays on the pasteboard.
    static let lifetime: TimeInterval = 120

    /// Copies text that never syncs to other devices (Universal Clipboard) and expires.
    static func copy(_ text: String) {
        #if canImport(UIKit)
        UIPasteboard.general.setItems(
            [[UTType.utf8PlainText.identifier: text]],
            options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(lifetime)]
        )
        #endif
    }
}

enum Platform {
    static func image(at path: String) -> Image? {
        #if canImport(UIKit)
        guard let ui = UIImage(contentsOfFile: path) else { return nil }
        return Image(uiImage: ui)
        #else
        return nil
        #endif
    }

    /// Posted when the system is about to end the app.
    static var willTerminate: Notification.Name {
        #if canImport(UIKit)
        UIApplication.willTerminateNotification
        #else
        Notification.Name("hush.willTerminate")
        #endif
    }
}

extension View {
    /// For links, codes and identifiers: no capitals, no corrections.
    func plainInput() -> some View {
        #if os(iOS)
        textInputAutocapitalization(.never).autocorrectionDisabled()
        #else
        autocorrectionDisabled()
        #endif
    }

    func hiddenNavigationBar() -> some View {
        #if os(iOS)
        toolbar(.hidden, for: .navigationBar)
        #else
        self
        #endif
    }

    func inlineTitle() -> some View {
        #if os(iOS)
        navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }
}
