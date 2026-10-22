import HushKit
import SwiftUI

struct IdentityView: View {
    @Environment(HushStore.self) private var store
    @State private var copied = false
    @State private var busy = false

    var body: some View {
        Form {
            if store.state.vault != .unlocked {
                VaultGate()
            } else if let me = store.state.identity {
                Section {
                    Text("idIntro").font(.footnote).foregroundStyle(.secondary)
                    QRCodeView(text: me, size: 200).frame(maxWidth: .infinity)
                    Text(verbatim: me)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                        .accessibilityIdentifier("my-id")
                    Button {
                        Clipboard.copy(me)
                        copied = true
                        Task {
                            try? await Task.sleep(nanoseconds: 1_500_000_000)
                            copied = false
                        }
                    } label: {
                        Label(copied ? LocalizedStringKey("copied") : "copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                    }
                    ShareLink(item: me) { Label("share", systemImage: "square.and.arrow.up") }
                }
                Section {
                    Toggle(isOn: Binding(get: { store.state.reachable }, set: { store.setReachable($0) })) {
                        Text("reachable")
                    }
                    .accessibilityIdentifier("reachable")
                } footer: {
                    Text("reachableHint")
                }
            } else {
                Section {
                    Text("idIntro").font(.footnote).foregroundStyle(.secondary)
                    Button {
                        busy = true
                        Task {
                            // The identifier arrives through the store's events; a failure leaves the
                            // button as it was.
                            _ = try? await store.ensureIdentity()
                            busy = false
                        }
                    } label: {
                        HStack {
                            if busy { ProgressView() }
                            Label("idCreate", systemImage: "key")
                        }
                    }
                    .disabled(busy)
                }
            }
            NameSection()
        }
        .navigationTitle("myId")
        .inlineTitle()
    }
}

struct NameSection: View {
    @Environment(HushStore.self) private var store

    var body: some View {
        Section {
            TextField("displayName", text: Binding(get: { store.settings.name }, set: { v in store.update { $0.name = String(v.prefix(64)) } }))
                .accessibilityIdentifier("name")
        } header: {
            Text("displayName")
        } footer: {
            Text("displayNameHint")
        }
    }
}
