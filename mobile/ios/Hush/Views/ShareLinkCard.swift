import HushKit
import SwiftUI

/// The link to send: QR code, text, share, copy, and how long it lasts.
struct ShareLinkCard: View {
    let url: String
    let expiresMs: UInt64?
    let group: Bool
    @State private var copied = false

    private var passphrase: Bool {
        (try? linkInfo(input: url))?.passphrase ?? false
    }

    var body: some View {
        VStack(spacing: 14) {
            Text("sendThisLink").font(.title3.bold())
            Text("sendThisLinkHint").font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
            QRCodeView(text: url, size: 208)
            // Not selectable: Copy below keeps it on this phone, for a while.
            Text(verbatim: url)
                .font(.footnote.monospaced())
                .multilineTextAlignment(.center)
                .accessibilityIdentifier("share-link")
            HStack {
                Button {
                    Clipboard.copy(url)
                    copied = true
                    Task {
                        try? await Task.sleep(nanoseconds: 1_500_000_000)
                        copied = false
                    }
                } label: {
                    Label(copied ? LocalizedStringKey("copied") : "copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                }
                .buttonStyle(.borderedProminent)
                if let link = URL(string: url) {
                    ShareLink(item: link) {
                        Label("share", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(.bordered)
                }
            }
            if let expiresMs {
                TimelineView(.periodic(from: .now, by: 30)) { ctx in
                    let left = Int64(expiresMs) - Int64(ctx.date.timeIntervalSince1970 * 1000)
                    Label {
                        Text("expiresIn \(Wording.duration(ms: left))") + Text(verbatim: " · ")
                            + Text(group ? LocalizedStringKey("anyoneWithLink") : "singleUse")
                    } icon: {
                        Image(systemName: "timer")
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
            }
            if passphrase {
                Text("sharePassphrase")
                    .font(.footnote)
                    .padding(10)
                    .frame(maxWidth: .infinity)
                    .background(RoundedRectangle(cornerRadius: 8).fill(.yellow.opacity(0.15)))
            }
        }
        .frame(maxWidth: .infinity)
    }
}
