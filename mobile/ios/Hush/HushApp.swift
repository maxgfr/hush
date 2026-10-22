import HushKit
import SwiftUI

@main
struct HushMain: App {
    @State private var store: HushStore?
    @State private var failure: String?
    @State private var router = Router()
    @Environment(\.scenePhase) private var phase

    init() {
        let info = ProcessInfo.processInfo
        // UI tests and the unit tests' host: a throwaway engine with no network and no radios.
        let ephemeral = info.arguments.contains("-ephemeral") || info.environment["XCTestConfigurationFilePath"] != nil
        do {
            _store = State(initialValue: try HushStore.start(.init(ephemeral: ephemeral)))
        } catch {
            _failure = State(initialValue: String(describing: error))
        }
    }

    var body: some Scene {
        WindowGroup {
            ZStack {
                if let store {
                    RootView()
                        .environment(store)
                        .environment(router)
                        .onReceive(NotificationCenter.default.publisher(for: Platform.willTerminate)) { _ in
                            // Tells the peers we left and erases the keys.
                            store.engine.shutdown()
                        }
                } else {
                    ContentUnavailableView("hush", systemImage: "exclamationmark.triangle", description: Text(failure ?? ""))
                }
                // Nothing of a conversation shows in the app switcher.
                if phase != .active {
                    PrivacyShield()
                }
            }
            .onOpenURL { url in router.open(url) }
        }
    }
}

struct PrivacyShield: View {
    var body: some View {
        ZStack {
            Rectangle().fill(.background).ignoresSafeArea()
            VStack(spacing: 12) {
                Image(systemName: "lock.shield.fill").font(.system(size: 56)).foregroundStyle(.tint)
                Text("hush").font(.largeTitle.bold())
            }
        }
        .accessibilityIdentifier("privacy-shield")
    }
}

struct RootView: View {
    @Environment(Router.self) private var router

    var body: some View {
        @Bindable var router = router
        NavigationStack(path: $router.path) {
            HomeView()
                .navigationDestination(for: Route.self) { route in
                    switch route {
                    case .new: NewConversationView()
                    case let .join(link): JoinView(initial: link)
                    case let .chat(id): ChatView(chatId: id)
                    case .contacts: ContactsView()
                    case .identity: IdentityView()
                    case .settings: SettingsView()
                    }
                }
        }
    }
}
