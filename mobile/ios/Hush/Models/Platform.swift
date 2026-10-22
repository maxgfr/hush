import HushKit
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
    /// What a saved file shows: read and drawn small away from the main thread, by its bytes.
    static func look(at path: String, mime: String) async -> (image: Image?, previewable: Bool) {
        let url = URL(fileURLWithPath: path)
        let look = await Task.detached(priority: .utility) { ReceivedFile.look(at: url, mime: mime) }.value
        return (look.thumbnail.map { Image(decorative: $0, scale: 1) }, look.previewable)
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

#if canImport(UIKit)
final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        PrivacyCurtain.shared.start()
        return true
    }

    /// Only the system keyboard: another one could keep or send what is typed.
    func application(_ application: UIApplication, shouldAllowExtensionPointIdentifier id: UIApplication.ExtensionPointIdentifier) -> Bool {
        id != .keyboard
    }
}

/// The privacy shield in a window of its own, above everything the app shows (sheets, Quick Look):
/// while the app is not in front, since the app switcher's snapshot is written to disk, and while
/// the screen is recorded or mirrored. iOS gives no way to hide anything from a screenshot.
@MainActor
final class PrivacyCurtain {
    static let shared = PrivacyCurtain()
    private var window: UIWindow?
    private var watched: Set<ObjectIdentifier> = []

    func start() {
        let center = NotificationCenter.default
        center.addObserver(forName: UIScene.willDeactivateNotification, object: nil, queue: .main) { note in
            MainActor.assumeIsolated { self.refresh(note.object as? UIWindowScene, active: false) }
        }
        center.addObserver(forName: UIScene.didActivateNotification, object: nil, queue: .main) { note in
            MainActor.assumeIsolated { self.refresh(note.object as? UIWindowScene, active: true) }
        }
    }

    private func refresh(_ scene: UIWindowScene?, active: Bool) {
        guard let scene else { return }
        if watched.insert(ObjectIdentifier(scene)).inserted {
            scene.registerForTraitChanges([UITraitSceneCaptureState.self]) { (scene: UIWindowScene, _: UITraitCollection) in
                self.refresh(scene, active: scene.activationState == .foregroundActive)
            }
        }
        let captured = scene.traitCollection.sceneCaptureState == .active
        if active && !captured {
            window?.isHidden = true
            window = nil
        } else if window?.windowScene !== scene {
            let w = UIWindow(windowScene: scene)
            w.windowLevel = .alert + 1
            w.rootViewController = UIHostingController(rootView: PrivacyShield())
            w.isHidden = false
            window = w
        }
    }
}
#endif
