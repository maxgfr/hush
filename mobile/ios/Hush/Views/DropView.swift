import HushKit
import SwiftUI
import UniformTypeIdentifiers

/// Send files by link: they are sealed on this phone, then wait on blob servers that only see
/// ciphertext.
struct DropView: View {
    @Environment(HushStore.self) private var store
    @State private var model = DropModel()
    @State private var importing = false

    var body: some View {
        @Bindable var model = model
        Form {
            if let url = model.url {
                Section {
                    ShareLinkCard(url: url, expiresMs: model.expiresMs, group: !model.once)
                    Text("dropReady").font(.footnote).foregroundStyle(.secondary)
                }
            } else if model.busy {
                Section { DropProgressView(model: model) }
            } else {
                Section {
                    Button {
                        importing = true
                    } label: {
                        Label("pickFiles", systemImage: "doc.badge.plus")
                    }
                    .accessibilityIdentifier("drop-pick")
                    ForEach(model.files, id: \.self) { f in
                        Label(f.lastPathComponent, systemImage: "doc")
                    }
                }
                Section {
                    Picker("linkExpiry", selection: $model.minutes) {
                        ForEach([UInt64(10), 60, 1440, 10080], id: \.self) { m in
                            Text(Wording.duration(ms: Int64(m) * 60_000)).tag(m)
                        }
                    }
                    Toggle("dropOnce", isOn: $model.once)
                    SecureField("passphrase", text: $model.passphrase)
                }
                Section {
                    Text("dropHint").font(.footnote).foregroundStyle(.secondary)
                    if let e = model.error { Text(e).foregroundStyle(.red) }
                    Button {
                        Task { await model.send(store) }
                    } label: {
                        Label("sealAndSend", systemImage: "lock")
                    }
                    .disabled(model.files.isEmpty)
                    .accessibilityIdentifier("drop-send")
                }
            }
        }
        .navigationTitle("sendFiles")
        .inlineTitle()
        .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case let .success(urls) = result else { return }
            model.files = Array(Set(model.files + urls))
        }
    }
}

/// A drop's sealed copy: downloaded and decrypted on this phone.
struct DropReceiveView: View {
    let link: String
    @Environment(HushStore.self) private var store
    @State private var model = DropModel()

    var body: some View {
        @Bindable var model = model
        Form {
            if !model.saved.isEmpty {
                Section {
                    if let s = model.sender { Text("dropFrom \(s)").font(.footnote) }
                    Label("dropSaved", systemImage: "checkmark.circle").foregroundStyle(.green)
                    ForEach(model.saved, id: \.self) { path in
                        ShareLink(item: URL(fileURLWithPath: path)) {
                            Label((path as NSString).lastPathComponent, systemImage: "doc")
                        }
                    }
                    if model.deleted { Text("dropDeleted").font(.footnote).foregroundStyle(.secondary) }
                }
            } else if model.busy {
                Section { DropProgressView(model: model) }
            } else {
                Section {
                    Text("dropReceiveHint").font(.footnote).foregroundStyle(.secondary)
                    if DropModel.needsPassphrase(link) {
                        SecureField("passphrase", text: $model.passphrase)
                    }
                    if let e = model.error { Text(e).foregroundStyle(.red) }
                    Button {
                        Task { await model.receive(link, store: store) }
                    } label: {
                        Label("downloadAll", systemImage: "arrow.down.circle")
                    }
                    .disabled(DropModel.needsPassphrase(link) && model.passphrase.isEmpty)
                    .accessibilityIdentifier("drop-download")
                }
            }
        }
        .navigationTitle("filesForYou")
        .inlineTitle()
    }
}

private struct DropProgressView: View {
    let model: DropModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label).font(.headline)
            ProgressView(value: model.total > 0 ? Double(model.done) / Double(model.total) : 0)
            if model.total > 0 {
                Text("\(Wording.size(model.done)) / \(Wording.size(model.total))").font(.footnote).foregroundStyle(.secondary)
            }
        }
    }

    private var label: LocalizedStringKey {
        switch model.phase {
        case .uploading: "dropUploading"
        case .downloading: "dropDownloading"
        case .sealing, nil: "dropSealing"
        }
    }
}
