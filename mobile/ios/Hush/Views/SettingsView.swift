import HushKit
import SwiftUI

struct SettingsView: View {
    @Environment(HushStore.self) private var store
    @State private var relays = ""
    @State private var trackers = ""
    @State private var stun = ""
    @State private var turnURL = ""
    @State private var turnUsername = ""
    @State private var turnCredential = ""
    @State private var saved = false

    var body: some View {
        Form {
            NameSection()
            Section {
                Picker("messageTimer", selection: Binding(get: { store.settings.ttl }, set: { v in store.update { $0.ttl = v } })) {
                    ForEach(Wording.ttlSeconds, id: \.self) { s in Text(Wording.ttl(s)).tag(s) }
                }
                Toggle(isOn: Binding(get: { store.settings.nearby }, set: { v in store.update { $0.nearby = v } })) {
                    Text("nearby")
                }
                .accessibilityIdentifier("nearby")
            } footer: {
                Text("nearbyHint")
            }
            Section {
                Text("networkHint").font(.footnote).foregroundStyle(.secondary)
                field("relays", text: $relays)
                field("trackers", text: $trackers)
                field("stun", text: $stun)
            } header: {
                Text("network")
            }
            Section {
                TextField(text: $turnURL, prompt: Text(verbatim: "turn:turn.example.org:3478")) { Text("turn") }
                    .plainInput()
                TextField(text: $turnUsername, prompt: Text("username")) { Text("username") }
                    .plainInput()
                SecureField(text: $turnCredential, prompt: Text("credential")) { Text("credential") }
                Toggle(isOn: Binding(get: { store.settings.relayOnly }, set: { v in store.update { $0.relayOnly = v } })) {
                    Text("relayOnly")
                }
            } header: {
                Text("turn")
            } footer: {
                Text("turnHint")
            }
            Section {
                Button(saved ? LocalizedStringKey("saved") : "save", action: save)
                Button("reset") {
                    let d = Settings.defaults
                    relays = d.relays.joined(separator: "\n")
                    trackers = d.trackers.joined(separator: "\n")
                    stun = d.stun.joined(separator: "\n")
                }
            }
        }
        .navigationTitle("settings")
        .inlineTitle()
        .onAppear(perform: load)
    }

    private func field(_ title: LocalizedStringKey, text: Binding<String>) -> some View {
        VStack(alignment: .leading) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            TextEditor(text: text)
                .font(.caption.monospaced())
                .frame(minHeight: 80)
                .plainInput()
        }
    }

    private func load() {
        let s = store.settings
        relays = s.relays.joined(separator: "\n")
        trackers = s.trackers.joined(separator: "\n")
        stun = s.stun.joined(separator: "\n")
        turnURL = s.turnURL
        turnUsername = s.turnUsername
        turnCredential = store.turnCredential
    }

    private func save() {
        store.update({
            $0.relays = Settings.lines(relays)
            $0.trackers = Settings.lines(trackers)
            $0.stun = Settings.lines(stun)
            $0.turnURL = turnURL.trimmingCharacters(in: .whitespaces)
            $0.turnUsername = turnUsername
        }, turnCredential: turnCredential)
        saved = true
        Task {
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            saved = false
        }
    }
}
