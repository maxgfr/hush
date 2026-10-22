import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI
#if canImport(UIKit)
import AVFoundation
import UIKit
#endif

/// A QR code drawn on the device (CoreImage): the link never leaves the app to be drawn.
struct QRCodeView: View {
    let text: String
    var size: CGFloat = 220

    var body: some View {
        Group {
            if let image = Self.image(text) {
                Image(decorative: image, scale: 1)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
            } else {
                Image(systemName: "qrcode").resizable().scaledToFit().foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 12).fill(.white))
        .accessibilityElement()
        .accessibilityLabel(Text("qrCode"))
        .accessibilityIdentifier("qr-code")
    }

    static func image(_ text: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let out = filter.outputImage else { return nil }
        let scaled = out.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        return CIContext().createCGImage(scaled, from: scaled.extent)
    }
}

#if canImport(UIKit)
/// The camera, reading QR codes (AVFoundation).
struct QRScannerView: UIViewControllerRepresentable {
    let onCode: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let c = ScannerController()
        c.onCode = onCode
        return c
    }

    func updateUIViewController(_ controller: ScannerController, context: Context) {}

    final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
        var onCode: ((String) -> Void)?
        private let session = AVCaptureSession()
        private var preview: AVCaptureVideoPreviewLayer?
        private var done = false

        override func viewDidLoad() {
            super.viewDidLoad()
            view.backgroundColor = .black
            AVCaptureDevice.requestAccess(for: .video) { granted in
                DispatchQueue.main.async { granted ? self.configure() : self.unavailable() }
            }
        }

        private func configure() {
            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input) else { return unavailable() }
            session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else { return unavailable() }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            output.metadataObjectTypes = [.qr]
            let layer = AVCaptureVideoPreviewLayer(session: session)
            layer.videoGravity = .resizeAspectFill
            layer.frame = view.bounds
            view.layer.addSublayer(layer)
            preview = layer
            let session = session
            DispatchQueue.global(qos: .userInitiated).async { session.startRunning() }
        }

        private func unavailable() {
            let label = UILabel()
            label.text = String(localized: "cameraUnavailable")
            label.textColor = .white
            label.numberOfLines = 0
            label.textAlignment = .center
            label.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(label)
            NSLayoutConstraint.activate([
                label.centerYAnchor.constraint(equalTo: view.centerYAnchor),
                label.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 24),
                label.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -24),
            ])
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            preview?.frame = view.bounds
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            let session = session
            DispatchQueue.global(qos: .userInitiated).async { session.stopRunning() }
        }

        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
            guard !done, let code = objects.compactMap({ $0 as? AVMetadataMachineReadableCodeObject }).first?.stringValue else { return }
            done = true
            onCode?(code)
        }
    }
}
#else
struct QRScannerView: View {
    let onCode: (String) -> Void
    var body: some View { Text("cameraUnavailable") }
}
#endif
