import HushKit
import PhotosUI
import QuickLook
import SwiftUI

struct ChatView: View {
    let chatId: UInt64
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router
    @State private var confirmEnd = false
    @State private var inviting = false

    static func title(_ c: ChatModel, contacts: [ContactEntry]) -> String {
        if c.group { return String(localized: "members \(c.peers.count + 1)") }
        if let key = c.contactKey, let contact = contacts.first(where: { $0.key == key }), !contact.name.isEmpty {
            return contact.name
        }
        if let p = c.peers.first, !p.name.isEmpty { return p.name }
        return c.isOpen ? String(localized: "them") : "hush"
    }

    var body: some View {
        Group {
            if let chat = store.chat(chatId) {
                if let reason = chat.ended {
                    EndedView(reason: reason) {
                        store.dismiss(chatId)
                        router.home()
                    }
                } else {
                    content(chat)
                }
            } else {
                ContentUnavailableView("ended", systemImage: "checkmark.circle")
            }
        }
        .onAppear { store.setVisible(chatId) }
        .onDisappear { store.setVisible(nil) }
    }

    private func content(_ chat: ChatModel) -> some View {
        // A link's conversation only ends with End; a contact's stays when leaving it.
        let isLink = chat.contactKey == nil
        return ScrollViewReader { scroll in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if !chat.isOpen {
                        WaitingPanel(chat: chat, passphrase: store.joinedWithPassphrase.contains(chatId))
                    }
                    if chat.locked {
                        NoticeRow(text: String(localized: "locked"))
                    }
                    ForEach(chat.peers.filter(\.needsCheck)) { peer in
                        VerifyPanel(chat: chat, peer: peer)
                    }
                    ForEach(chat.offers) { offer in
                        OfferBanner(chat: chat, offer: offer)
                    }
                    if chat.isOpen {
                        NoticeRow(text: String(localized: "newSessionStarted"), icon: "lock")
                    }
                    ForEach(chat.items) { item in
                        MessageRow(chat: chat, item: item).id(item.id)
                    }
                }
                .padding()
            }
            .onChange(of: chat.items.count) {
                if let last = chat.items.last { withAnimation { scroll.scrollTo(last.id, anchor: .bottom) } }
            }
        }
        .safeAreaInset(edge: .bottom) {
            if chat.isOpen { Composer(chatId: chatId) }
        }
        .navigationTitle(Self.title(chat, contacts: store.state.contacts))
        .inlineTitle()
        .navigationBarBackButtonHidden(isLink)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 2) {
                    Text(Self.title(chat, contacts: store.state.contacts)).font(.headline)
                    HStack(spacing: 6) {
                        if !chat.group, let p = chat.peers.first { PeerBadge(peer: p) }
                        Text("e2ePq").font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
            ToolbarItem(placement: .primaryAction) {
                menu(chat)
            }
            if isLink {
                ToolbarItem(placement: .cancellationAction) {
                    Button("endConversation", role: .destructive) {
                        if chat.peers.isEmpty { store.end(chatId) } else { confirmEnd = true }
                    }
                    .tint(.red)
                    .accessibilityIdentifier("end")
                }
            }
        }
        .confirmationDialog("endConversation", isPresented: $confirmEnd, titleVisibility: .visible) {
            Button("endConversation", role: .destructive) { store.end(chatId) }
        } message: {
            Text("endConfirm")
        }
        .sheet(isPresented: $inviting) {
            NavigationStack {
                ScrollView {
                    if let url = chat.url { ShareLinkCard(url: url, expiresMs: chat.expiresMs, group: true).padding() }
                }
                .navigationTitle("invite")
                .inlineTitle()
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) { Button("close") { inviting = false } }
                }
            }
        }
    }

    private func menu(_ chat: ChatModel) -> some View {
        Menu {
            Picker(selection: Binding(get: { store.settings.ttl }, set: { v in store.update { $0.ttl = v } })) {
                ForEach(Wording.ttlSeconds, id: \.self) { s in Text(Wording.ttl(s)).tag(s) }
            } label: {
                Label("messageTimer", systemImage: "timer")
            }
            if chat.group {
                if chat.url != nil && chat.isOpen {
                    Button { inviting = true } label: { Label("invite", systemImage: "link") }
                }
                Button {
                    store.setLocked(chatId, !chat.locked)
                } label: {
                    Label(chat.locked ? LocalizedStringKey("unlock") : "lock", systemImage: chat.locked ? "lock.open" : "lock")
                }
            }
            if chat.contactKey == nil {
                ForEach(chat.peers) { p in
                    Button {
                        store.saveContact(chatId, peer: p.id)
                    } label: {
                        Label(chat.group ? "\(String(localized: "saveContact")) · \(p.name)" : String(localized: "saveContact"), systemImage: "person.badge.plus")
                    }
                }
            } else {
                Button(role: .destructive) {
                    store.end(chatId)
                    router.path = [.contacts]
                } label: {
                    Label("endConversation", systemImage: "rectangle.portrait.and.arrow.right")
                }
            }
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .accessibilityLabel(Text("menu"))
    }
}

struct WaitingPanel: View {
    let chat: ChatModel
    let passphrase: Bool

    var body: some View {
        VStack(spacing: 16) {
            if chat.creator, let url = chat.url {
                ShareLinkCard(url: url, expiresMs: chat.expiresMs, group: chat.group)
            }
            HStack(spacing: 8) {
                ProgressView()
                Text(status).accessibilityIdentifier("status")
            }
            .font(.callout)
            if !chat.creator && passphrase && chat.status != .connecting {
                Text("waitingPassHint").font(.footnote).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var status: LocalizedStringKey {
        if chat.status == .connecting { return "connecting" }
        return chat.creator || chat.contactKey != nil ? "waitingPeer" : "waitingCreator"
    }
}

struct PeerBadge: View {
    let peer: PeerModel

    var body: some View {
        HStack(spacing: 4) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(Wording.path(peer.path))
            if peer.verified {
                Image(systemName: "checkmark.shield.fill").foregroundStyle(.green)
                Text("verified")
            }
        }
        .font(.caption2)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("peer-state")
    }

    private var color: Color {
        switch peer.path {
        case .connecting: .orange
        case .relayed: .yellow
        default: .green
        }
    }
}

struct VerifyPanel: View {
    let chat: ChatModel
    let peer: PeerModel
    @Environment(HushStore.self) private var store

    var body: some View {
        let name = peer.name.isEmpty ? String(localized: "them") : peer.name
        VStack(alignment: .leading, spacing: 12) {
            Label {
                Text("verifyTitle") + Text(verbatim: chat.group ? " · \(name)" : "")
            } icon: {
                Image(systemName: "checkmark.shield")
            }
            .font(.headline)
            Text("verifyHint \(name)").font(.footnote).foregroundStyle(.secondary)
            HStack(alignment: .top) {
                ForEach(Array(Wording.sas(peer.sas).enumerated()), id: \.offset) { _, e in
                    VStack(spacing: 4) {
                        Text(verbatim: e.emoji).font(.system(size: 34))
                        Text(verbatim: e.name).font(.caption2).multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .accessibilityIdentifier("sas")
            if peer.verifiedByMe {
                Text("waitingTheirCheck \(name)").font(.footnote).foregroundStyle(.secondary)
            } else {
                HStack {
                    Button {
                        store.verify(chat.id, peer: peer.id, ok: true)
                    } label: {
                        Label("match", systemImage: "checkmark").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("sas-match")
                    Button(role: .destructive) {
                        store.verify(chat.id, peer: peer.id, ok: false)
                    } label: {
                        Label("noMatch", systemImage: "xmark").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
            }
        }
        .padding()
        .background(RoundedRectangle(cornerRadius: 12).fill(.tint.opacity(0.08)))
    }
}

struct OfferBanner: View {
    let chat: ChatModel
    let offer: CardOfferModel
    @Environment(HushStore.self) private var store
    @Environment(Router.self) private var router

    var body: some View {
        let name = offer.name.isEmpty ? String(localized: "them") : offer.name
        HStack {
            Text("contactAsked \(name)").font(.callout)
            Spacer()
            Button("saveContact") {
                if store.state.vault == .unlocked {
                    store.saveContact(chat.id, peer: offer.peer)
                } else {
                    router.path.append(.contacts)
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)
            Button {
                store.dismissOffer(chat.id, peer: offer.peer)
            } label: {
                Image(systemName: "xmark")
            }
            .accessibilityLabel(Text("close"))
        }
        .padding()
        .background(RoundedRectangle(cornerRadius: 12).fill(.blue.opacity(0.08)))
    }
}

struct NoticeRow: View {
    let text: String
    var icon: String?

    var body: some View {
        HStack(spacing: 4) {
            if let icon { Image(systemName: icon) }
            Text(verbatim: text)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity)
        .multilineTextAlignment(.center)
    }
}

struct MessageRow: View {
    let chat: ChatModel
    let item: ChatItem
    @Environment(HushStore.self) private var store

    var body: some View {
        switch item.body {
        case let .notice(n):
            NoticeRow(text: Wording.notice(n))
        case let .text(text):
            bubble { Text(verbatim: text).textSelection(.enabled) }
                .contextMenu {
                    Button {
                        Clipboard.copy(text)
                    } label: {
                        Label("copy", systemImage: "doc.on.doc")
                    }
                    if item.mine { deleteButton }
                }
        case let .file(f):
            bubble { FileCard(chatId: chat.id, file: f) }
                .contextMenu { if item.mine { deleteButton } }
        }
    }

    private var deleteButton: some View {
        Button(role: .destructive) {
            store.delete(item.id, in: chat.id)
        } label: {
            Label("deleteMessage", systemImage: "trash")
        }
    }

    private func bubble(@ViewBuilder _ content: () -> some View) -> some View {
        HStack {
            if item.mine { Spacer(minLength: 40) }
            VStack(alignment: item.mine ? .trailing : .leading, spacing: 2) {
                if chat.group && !item.mine {
                    Text(verbatim: item.author.isEmpty ? String(localized: "someone") : item.author)
                        .font(.caption.bold()).foregroundStyle(.secondary)
                }
                VStack(alignment: .leading, spacing: 4) {
                    content()
                    HStack(spacing: 3) {
                        if item.expiresAt != nil { Image(systemName: "timer") }
                        Text(Date(timeIntervalSince1970: Double(item.ts) / 1000), style: .time)
                    }
                    .font(.caption2)
                    .opacity(0.7)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .foregroundStyle(item.mine ? Color.white : Color.primary)
                .background(RoundedRectangle(cornerRadius: 16).fill(item.mine ? AnyShapeStyle(.tint) : AnyShapeStyle(.quaternary)))
            }
            .accessibilityIdentifier("message")
            if !item.mine { Spacer(minLength: 40) }
        }
    }
}

struct FileCard: View {
    let chatId: UInt64
    let file: FileModel
    @Environment(HushStore.self) private var store
    @State private var preview: URL?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if case let .saved(path) = file.state, file.mime.hasPrefix("image/"), let image = Platform.image(at: path) {
                image.resizable().scaledToFit().frame(maxWidth: 240, maxHeight: 240).clipShape(RoundedRectangle(cornerRadius: 8))
            }
            HStack {
                Image(systemName: "doc")
                VStack(alignment: .leading) {
                    Text(verbatim: file.name).font(.callout.bold()).lineLimit(2)
                    Text(verbatim: Wording.size(file.size)).font(.caption)
                }
            }
            switch file.state {
            case .offered:
                Button { store.download(file.id, in: chatId) } label: { Label("download", systemImage: "arrow.down.circle") }
                    .buttonStyle(.bordered)
            case let .receiving(done, total):
                ProgressView(value: Double(done), total: Double(max(total, 1)))
                HStack {
                    Text("downloading \(Int(Double(done) * 100 / Double(max(total, 1))))").font(.caption)
                    Spacer()
                    Button("cancel", role: .destructive) { store.cancel(file.id, in: chatId) }.font(.caption)
                }
            case let .saved(path):
                let url = URL(fileURLWithPath: path)
                HStack {
                    Button { preview = url } label: { Label("open", systemImage: "eye") }
                    ShareLink(item: url) { Label("share", systemImage: "square.and.arrow.up") }
                }
                .buttonStyle(.bordered)
            case .sent:
                Text("offered").font(.caption)
            case .delivered:
                Label("delivered", systemImage: "checkmark").font(.caption)
            case let .failed(reason):
                Text(reason.contains("direct") ? LocalizedStringKey("directNeeded") : reason == "cancelled" ? LocalizedStringKey("cancelled") : "fileFailed")
                    .font(.caption).foregroundStyle(.red)
            }
        }
        .quickLookPreview($preview)
    }
}

struct Composer: View {
    let chatId: UInt64
    @Environment(HushStore.self) private var store
    @State private var text = ""
    @State private var importing = false
    @State private var photo: PhotosPickerItem?
    @State private var failed = false

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            Menu {
                Button { importing = true } label: { Label("files", systemImage: "folder") }
                PhotosPicker(selection: $photo, matching: .any(of: [.images, .videos])) {
                    Label("photos", systemImage: "photo")
                }
            } label: {
                Image(systemName: "paperclip").font(.title3)
            }
            .accessibilityLabel(Text("attach"))
            TextField("messagePlaceholder", text: $text, axis: .vertical)
                .lineLimit(1...5)
                .textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("composer")
            Button {
                store.send(text, in: chatId)
                text = ""
            } label: {
                Image(systemName: "arrow.up.circle.fill").font(.title2)
            }
            .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityLabel(Text("send"))
            .accessibilityIdentifier("send")
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
        .background(.bar)
        .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case let .success(urls) = result else { return }
            for url in urls {
                do { try store.sendFile(url, in: chatId) } catch { failed = true }
            }
        }
        .onChange(of: photo) {
            guard let item = photo else { return }
            photo = nil
            Task {
                guard let data = try? await item.loadTransferable(type: Data.self) else { return }
                let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
                let name = "photo-\(Int(Date().timeIntervalSince1970)).\(ext)"
                do { try store.sendFile(data, name: name, in: chatId) } catch { failed = true }
            }
        }
        .alert("fileFailed", isPresented: $failed) {
            Button("close", role: .cancel) {}
        }
    }
}

struct EndedView: View {
    let reason: Ending
    let done: () -> Void
    @Environment(HushStore.self) private var store

    var body: some View {
        VStack(spacing: 18) {
            Spacer()
            Image(systemName: reason == .taken || reason == .mismatch ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                .font(.system(size: 56))
                .foregroundStyle(reason == .taken || reason == .mismatch ? .orange : .green)
            Text(verbatim: Wording.endTitle(reason)).font(.title2.bold()).multilineTextAlignment(.center)
                .accessibilityIdentifier("ended-title")
            if reason == .taken {
                Text("takenHint")
                    .padding()
                    .background(RoundedRectangle(cornerRadius: 10).fill(.orange.opacity(0.12)))
            }
            Text(store.state.vault == .unlocked ? LocalizedStringKey("wipedKept") : "wiped")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button(action: done) {
                Text("startOver").bold().frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            Spacer()
        }
        .padding()
        .navigationBarBackButtonHidden()
    }
}
