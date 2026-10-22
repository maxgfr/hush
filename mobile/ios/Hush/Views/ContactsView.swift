import HushKit
import SwiftUI
import UniformTypeIdentifiers

/// Create or unlock the vault before anything that needs to be kept.
struct VaultGate: View {
    @Environment(HushStore.self) private var store
    @State private var model = VaultModel()

    var body: some View {
        @Bindable var model = model
        let creating = store.state.vault == .none
        Section {
            Label("vaultTitle", systemImage: "lock").font(.headline)
            if creating { Text("vaultIntro").font(.footnote).foregroundStyle(.secondary) }
            SecureField("vaultPassword", text: $model.passphrase)
                .textContentType(creating ? .newPassword : .password)
                .accessibilityIdentifier("vault-pass")
            if creating {
                SecureField("vaultPasswordConfirm", text: $model.confirm)
                    .textContentType(.newPassword)
                    .accessibilityIdentifier("vault-confirm")
            }
            if let e = model.error { Text(verbatim: e).foregroundStyle(.red) }
            Button {
                Task { _ = await model.submit(store, creating: creating) }
            } label: {
                HStack {
                    if model.busy { ProgressView() }
                    Text(model.busy ? LocalizedStringKey("vaultWorking") : creating ? "vaultCreate" : "vaultUnlock").bold()
                }
            }
            .disabled(model.busy || model.passphrase.isEmpty)
        } footer: {
            if creating { Text("vaultPasswordHint") }
        }
    }
}

struct ContactsView: View {
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router
    @State private var renaming: ContactEntry?
    @State private var newName = ""
    @State private var removing: ContactEntry?
    @State private var erasing = false
    /// A request we accepted: open its conversation when it comes.
    @State private var awaitingChat = false

    var body: some View {
        Form {
            if store.state.vault != .unlocked {
                VaultGate()
            } else {
                requests
                contacts
                if store.state.identity != nil { AddByIdSection() }
                BackupSection()
                Section {
                    Button("vaultDelete", role: .destructive) { erasing = true }
                }
            }
        }
        .navigationTitle("contacts")
        .inlineTitle()
        .toolbar {
            if store.state.vault == .unlocked {
                ToolbarItem(placement: .primaryAction) {
                    Button { store.lockVault() } label: { Label("vaultLock", systemImage: "lock") }
                }
            }
        }
        .alert("rename", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("displayName", text: $newName)
            Button("save") {
                if let c = renaming { store.rename(c.key, to: newName) }
                renaming = nil
            }
            Button("cancel", role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            removing.map { String(localized: "removeContactConfirm \($0.name)") } ?? "",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible
        ) {
            Button("removeContact", role: .destructive) {
                if let c = removing {
                    if let chat = c.chat { store.end(chat) }
                    store.remove(c.key)
                }
                removing = nil
            }
        }
        .confirmationDialog("vaultDeleteConfirm", isPresented: $erasing, titleVisibility: .visible) {
            Button("vaultDelete", role: .destructive) {
                Task { try? await store.eraseVault() }
            }
        }
        .onChange(of: store.state.order) { old, new in
            guard awaitingChat, let id = new.last, !old.contains(id) else { return }
            awaitingChat = false
            router.path.append(.chat(id))
        }
    }

    @ViewBuilder private var requests: some View {
        if !store.state.requests.isEmpty {
            Section("requests") {
                ForEach(store.state.requests, id: \.id) { r in
                    VStack(alignment: .leading, spacing: 8) {
                        Text("wantsToConnect \(r.name.isEmpty ? "?" : r.name) \(String(r.id.prefix(14)) + "…")")
                        HStack {
                            Button("accept") {
                                awaitingChat = true
                                store.accept(r.id)
                            }
                            .buttonStyle(.borderedProminent)
                            Button("decline") { store.decline(r.id) }.buttonStyle(.bordered)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder private var contacts: some View {
        Section("contacts") {
            if store.state.contacts.isEmpty {
                Text("noContacts").foregroundStyle(.secondary)
            }
            ForEach(store.state.contacts.sorted { $0.name.localizedCompare($1.name) == .orderedAscending }, id: \.key) { c in
                Button {
                    Task {
                        if let chat = try? await store.chatWith(c.key) { router.path.append(.chat(chat)) }
                    }
                } label: {
                    HStack {
                        Circle().fill(c.online ? Color.green : Color.gray.opacity(0.4)).frame(width: 9, height: 9)
                        VStack(alignment: .leading) {
                            Text(verbatim: c.name.isEmpty ? String(c.id.prefix(14)) + "…" : c.name)
                            Text(c.online ? LocalizedStringKey("online") : "offlineContact").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if let chat = c.chat, let unread = store.chat(chat)?.unread, unread > 0 {
                            Text("\(unread)").font(.caption.bold())
                        }
                    }
                }
                .contextMenu {
                    Button {
                        newName = c.name
                        renaming = c
                    } label: { Label("rename", systemImage: "pencil") }
                    Toggle(isOn: Binding(get: { c.history }, set: { store.setHistory(c.key, $0) })) {
                        Label("history", systemImage: "clock.arrow.circlepath")
                    }
                    Button {
                        Clipboard.copy(c.id)
                    } label: { Label("copy", systemImage: "doc.on.doc") }
                    Button(role: .destructive) { removing = c } label: { Label("removeContact", systemImage: "trash") }
                }
                .swipeActions {
                    Button(role: .destructive) { removing = c } label: { Label("removeContact", systemImage: "trash") }
                }
            }
        }
        if !store.state.contacts.isEmpty {
            Section {
                Text("contactsHint").font(.footnote).foregroundStyle(.secondary)
            }
        }
    }
}

struct AddByIdSection: View {
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router
    @State private var model = AddByIdModel()

    var body: some View {
        @Bindable var model = model
        let status = model.status(in: store.state)
        Section {
            TextField("idPlaceholder", text: $model.input)
                .plainInput()
                .font(.callout.monospaced())
                .accessibilityIdentifier("add-id")
            switch status?.status {
            case .searching:
                HStack { ProgressView(); Text("searching \(String(model.pending?.prefix(14) ?? "") + "…")") }
            case .requested:
                HStack { ProgressView(); Text("requestSent") }
            case .notFound:
                Text("notFound").foregroundStyle(.red)
            case .declined:
                Text("declined").foregroundStyle(.red)
            case nil:
                EmptyView()
            }
            if model.invalid { Text("invalidId").foregroundStyle(.red) }
            Button("connect") { model.submit(store) }
                .disabled(model.input.trimmingCharacters(in: .whitespaces).isEmpty || status?.status == .searching)
        } header: {
            Text("addById")
        } footer: {
            Text("addByIdHint")
        }
        .onChange(of: status?.chat) {
            if let chat = status?.chat { router.path.append(.chat(chat)) }
        }
    }
}

/// An encrypted backup file, as the web app and the CLI read it.
struct BackupFile: FileDocument {
    static let readableContentTypes: [UTType] = [.data]
    let data: Data

    init(data: Data) {
        self.data = data
    }

    init(configuration: ReadConfiguration) throws {
        data = configuration.file.regularFileContents ?? Data()
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: data)
    }
}

struct BackupSection: View {
    @Environment(HushStore.self) private var store
    @State private var passphrase = ""
    @State private var confirm = ""
    @State private var busy = false
    @State private var message: (ok: Bool, text: String)?
    @State private var exported: BackupFile?
    @State private var importing = false

    var body: some View {
        Section {
            Text("backupIntro").font(.footnote).foregroundStyle(.secondary)
            SecureField("backupPassword", text: $passphrase)
            // The file can be copied anywhere and attacked offline: as long a passphrase as the vault's.
            SecureField("vaultPasswordConfirm", text: $confirm)
            Button {
                if let problem = VaultModel.problem(passphrase, confirm: confirm) {
                    message = (false, problem)
                    return
                }
                run {
                    exported = BackupFile(data: try await store.exportBackup(passphrase))
                }
            } label: { Label("backupExport", systemImage: "square.and.arrow.down") }
                .disabled(passphrase.isEmpty || busy)
            Button { importing = true } label: { Label("backupImport", systemImage: "square.and.arrow.up") }
                .disabled(passphrase.isEmpty || busy)
            if let m = message {
                Text(verbatim: m.text).foregroundStyle(m.ok ? .green : .red)
            }
        } header: {
            Text("backup")
        } footer: {
            Text("backupPasswordHint")
        }
        .fileExporter(
            isPresented: Binding(get: { exported != nil }, set: { if !$0 { exported = nil } }),
            document: exported,
            contentType: .data,
            defaultFilename: "hush-backup-\(Date().formatted(.iso8601.year().month().day())).hush"
        ) { _ in exported = nil }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.data, .item]) { result in
            guard case let .success(url) = result else { return }
            run {
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let data = try Data(contentsOf: url)
                do {
                    let n = try await store.importBackup(data, passphrase: passphrase)
                    message = (true, String(localized: "backupDone \(Int(n))"))
                } catch {
                    message = (false, String(localized: "backupBad"))
                }
            }
        }
    }

    private func run(_ work: @escaping () async throws -> Void) {
        busy = true
        message = nil
        Task {
            do { try await work() } catch { message = (false, String(localized: "backupBad")) }
            busy = false
        }
    }
}
