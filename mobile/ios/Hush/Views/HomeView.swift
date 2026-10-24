import HushKit
import SwiftUI

struct HomeView: View {
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router

    var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 12) {
                    Label {
                        Text("hush").font(.largeTitle.bold())
                    } icon: {
                        Image(systemName: "lock.shield.fill").foregroundStyle(.tint)
                    }
                    Text("tagline").font(.title2.weight(.semibold))
                    Text("intro").foregroundStyle(.secondary)
                }
                .padding(.vertical, 8)
                NavigationLink(value: Route.new) {
                    Label("newConversation", systemImage: "link")
                }
                .accessibilityIdentifier("new-conversation")
                NavigationLink(value: Route.drop) {
                    Label("sendFiles", systemImage: "paperplane")
                }
                .accessibilityIdentifier("send-files")
                NavigationLink(value: Route.join(nil)) {
                    Label("joinWithLink", systemImage: "qrcode.viewfinder")
                }
                .accessibilityIdentifier("join")
            }

            let live = store.state.liveChats
            if !live.isEmpty {
                Section("conversations") {
                    ForEach(live) { c in
                        NavigationLink(value: Route.chat(c.id)) {
                            HStack {
                                Text(ChatView.title(c, contacts: store.state.contacts))
                                Spacer()
                                if c.unread > 0 {
                                    Text("\(c.unread)").font(.caption.bold()).padding(.horizontal, 6).padding(.vertical, 2)
                                        .background(Capsule().fill(.tint)).foregroundStyle(.white)
                                }
                            }
                        }
                    }
                }
            }

            Section {
                NavigationLink(value: Route.contacts) {
                    HStack {
                        Label("contacts", systemImage: "person.2")
                        Spacer()
                        switch store.state.vault {
                        case .unlocked where !store.state.contacts.isEmpty:
                            Text("\(store.state.contacts.count)").foregroundStyle(.secondary)
                        case .locked:
                            Image(systemName: "lock").foregroundStyle(.secondary)
                        default:
                            EmptyView()
                        }
                    }
                }
                NavigationLink(value: Route.identity) {
                    Label("myId", systemImage: "key")
                }
                NavigationLink(value: Route.settings) {
                    Label("settings", systemImage: "gearshape")
                }
            }

            Section {
                Label("X25519 + ML-KEM-768 · Ed25519 + ML-DSA-65", systemImage: "checkmark.shield")
                Label("noServer", systemImage: "globe")
                Link(destination: URL(string: "https://github.com/maxgfr/hush/blob/main/SECURITY.md")!) {
                    Label("security", systemImage: "doc.text")
                }
                Link(destination: URL(string: "https://github.com/maxgfr/hush")!) {
                    Label("source", systemImage: "chevron.left.forwardslash.chevron.right")
                }
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
        .navigationTitle("hush")
        .hiddenNavigationBar()
    }
}
