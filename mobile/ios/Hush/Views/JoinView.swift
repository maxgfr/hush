import HushKit
import SwiftUI

struct JoinView: View {
    let initial: String?
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router
    @State private var model = JoinModel()
    @State private var scanning = false

    var body: some View {
        @Bindable var model = model
        Form {
            if model.askPassphrase {
                Section {
                    Label("protectedLink", systemImage: "lock").font(.headline)
                    Text("protectedLinkHint").font(.footnote).foregroundStyle(.secondary)
                    SecureField("passphraseShort", text: $model.passphrase)
                        .accessibilityIdentifier("join-passphrase")
                        .onSubmit(join)
                }
                Section {
                    button(model.busy ? "deriving" : "join", action: join)
                        .disabled(model.passphrase.isEmpty || model.busy)
                }
            } else {
                Section("pasteLink") {
                    TextField("https://maxgfr.github.io/hush/#2.s.…", text: $model.input, axis: .vertical)
                        .lineLimit(3...6)
                        .plainInput()
                        .font(.callout.monospaced())
                        .accessibilityIdentifier("join-link")
                    PasteButton(payloadType: String.self) { texts in
                        if let t = texts.first { model.input = t }
                    }
                }
                if let p = model.problemText {
                    Section { Text(p).foregroundStyle(.red) }
                }
                Section {
                    button(model.busy ? "deriving" : "join") { submit(model.input) }
                        .disabled(model.input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.busy)
                    Button {
                        scanning = true
                    } label: {
                        Label("scanQr", systemImage: "qrcode.viewfinder")
                    }
                }
            }
        }
        .navigationTitle("joinWithLink")
        .inlineTitle()
        .sheet(isPresented: $scanning) {
            NavigationStack {
                QRScannerView { text in
                    scanning = false
                    model.input = text
                    submit(text)
                }
                .ignoresSafeArea()
                .navigationTitle("scanQr")
                .inlineTitle()
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("cancel") { scanning = false }
                    }
                }
            }
        }
        .task {
            if let initial, model.input.isEmpty { model.prefill(initial) }
        }
    }

    private func button(_ title: LocalizedStringKey, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack {
                Spacer()
                if model.busy { ProgressView().padding(.trailing, 4) }
                Text(title).bold()
                Spacer()
            }
        }
    }

    private func submit(_ text: String) {
        Task {
            if let chat = await model.submit(text, store: store) { router.show(chat: chat) }
        }
    }

    private func join() {
        guard !model.passphrase.isEmpty else { return }
        Task {
            if let chat = await model.join(store) { router.show(chat: chat) }
        }
    }
}
