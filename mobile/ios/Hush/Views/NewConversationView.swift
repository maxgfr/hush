import HushKit
import SwiftUI

struct NewConversationView: View {
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router
    @State private var model = NewConversationModel()

    var body: some View {
        @Bindable var model = model
        Form {
            Section {
                Picker("newConversation", selection: $model.group) {
                    Label("kindDirect", systemImage: "person").tag(false)
                    Label("kindGroup", systemImage: "person.3").tag(true)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                Text(model.group ? LocalizedStringKey("kindGroupHint") : "kindDirectHint")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            Section {
                Picker("linkExpiry", selection: $model.minutes) {
                    ForEach(Wording.linkMinutes, id: \.self) { m in
                        Text(Wording.duration(ms: Int64(m) * 60_000)).tag(m)
                    }
                }
                Picker("messageTimer", selection: Binding(
                    get: { store.settings.ttl },
                    set: { v in store.update { $0.ttl = v } }
                )) {
                    ForEach(Wording.ttlSeconds, id: \.self) { s in
                        Text(Wording.ttl(s)).tag(s)
                    }
                }
            }
            Section {
                SecureField("passphrase", text: $model.passphrase)
                    .textContentType(.newPassword)
                    .accessibilityIdentifier("link-passphrase")
            } footer: {
                Text("passphraseHint")
            }
            Section {
                Button {
                    Task {
                        if let chat = await model.create(store) { router.show(chat: chat) }
                    }
                } label: {
                    HStack {
                        Spacer()
                        if model.busy { ProgressView().padding(.trailing, 4) }
                        Text(model.busy && !model.passphrase.isEmpty ? LocalizedStringKey("deriving") : "createLink").bold()
                        Spacer()
                    }
                }
                .disabled(model.busy)
                .accessibilityIdentifier("create-link")
                if let e = model.error {
                    Text(e).foregroundStyle(.red).font(.footnote)
                }
            }
        }
        .navigationTitle("newConversation")
        .inlineTitle()
    }
}
