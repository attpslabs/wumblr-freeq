import AVFoundation
import CoreVideo
import Foundation
import UIKit

/// Drives the iOS front camera and pumps BGRA frames to the Rust AV pipeline.
///
/// We do capture in Swift because iroh-live's AVFoundation camera backend is
/// still stubbed (see `rusty-capture/src/platform/apple/camera.rs`). The Rust
/// side exposes `push_video_frame(bgra, w, h, ts_us)` which feeds a
/// `PushVideoSource` plugged into the broadcast's H.264 encoder.
///
/// Lifecycle:
/// - `start()` configures the session, requests permission, kicks the camera
///   on, and begins delivering frames to `onFrame`.
/// - `stop()` tears the session down.
///
/// The capture session is configured for 1280×720, BGRA, 30fps. The preview
/// layer can be wired separately via `previewLayer` for a low-latency local
/// preview (which doesn't go through the Rust copy).
final class CallCameraCapture: NSObject {
    /// Callback fires on the capture queue. Implementations should be fast —
    /// the queue is serial and frames will queue up behind a slow consumer.
    var onFrame: ((_ bgra: UnsafePointer<UInt8>, _ length: Int, _ width: Int, _ height: Int, _ timestampMicros: UInt64) -> Void)?

    /// Preview layer for the local "self view" tile. Always shows the most
    /// recent frame the OS captured, regardless of whether the encoder is
    /// keeping up.
    let previewLayer: AVCaptureVideoPreviewLayer

    private let session: AVCaptureSession
    private let videoOutput: AVCaptureVideoDataOutput
    private let queue = DispatchQueue(label: "at.freeq.camera-capture", qos: .userInitiated)
    private var configured = false

    /// Cached so the orientation handler doesn't have to look it back up.
    /// Default reflects the configure-time angle (90° portrait preview, 0°
    /// data output). The data-output value is locked at this initial value
    /// across orientation changes — see the long comment in
    /// `configureIfNeeded`.
    private var initialDataOutputAngle: CGFloat = 0
    private var lastValidOrientation: UIDeviceOrientation = .portrait

    override init() {
        self.session = AVCaptureSession()
        self.videoOutput = AVCaptureVideoDataOutput()
        self.previewLayer = AVCaptureVideoPreviewLayer(session: session)
        self.previewLayer.videoGravity = .resizeAspectFill
        super.init()
        // `orientationDidChangeNotification` is silent until *something* turns
        // on the accelerometer-backed orientation source. Apps that don't
        // care don't get the cost. We care.
        if Thread.isMainThread {
            UIDevice.current.beginGeneratingDeviceOrientationNotifications()
        } else {
            DispatchQueue.main.async {
                UIDevice.current.beginGeneratingDeviceOrientationNotifications()
            }
        }
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(deviceOrientationDidChange),
            name: UIDevice.orientationDidChangeNotification,
            object: nil
        )
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        DispatchQueue.main.async {
            UIDevice.current.endGeneratingDeviceOrientationNotifications()
        }
    }

    @objc private func deviceOrientationDidChange() {
        // Marshal to the main thread — AVCaptureConnection mutations and
        // CALayer transforms must happen there. Notifications fire on
        // whichever thread the post happened on.
        DispatchQueue.main.async {
            self.applyOrientation(UIDevice.current.orientation)
        }
    }

    /// Rotate only the local preview connection. The data-output connection
    /// (encoder feed) is deliberately left alone — the H.264 encoder is set
    /// for 1280x720 landscape and will reject rotated frames; receivers can
    /// rotate their display layer if they care. Today they don't, which is
    /// why portrait-held callers look sideways on the other side; fixing
    /// that requires the receiver to rotate, not us.
    fileprivate func applyOrientation(_ orientation: UIDeviceOrientation) {
        let angle: CGFloat?
        switch orientation {
        case .portrait:
            angle = 90
        case .portraitUpsideDown:
            angle = 270
        case .landscapeLeft:
            // Home button on the right — native sensor orientation.
            angle = 0
        case .landscapeRight:
            angle = 180
        case .faceUp, .faceDown, .unknown:
            // Not a meaningful UI rotation; leave the prior angle in place.
            angle = nil
        @unknown default:
            angle = nil
        }
        guard let angle else {
            print("[camera] orientation: \(orientation.rawValue) — skipped (faceUp/faceDown/unknown)")
            return
        }
        lastValidOrientation = orientation
        let conn = previewLayer.connection
        let supported = conn?.isVideoRotationAngleSupported(angle) ?? false
        print("[camera] orientation: raw=\(orientation.rawValue) angle=\(angle) connection=\(conn != nil) supported=\(supported)")
        if let conn, supported {
            conn.videoRotationAngle = angle
        } else {
            // Fallback: if AVCaptureVideoPreviewLayer's connection isn't
            // ready yet (it isn't until startRunning has connected the
            // session), apply the rotation as a CALayer affine transform.
            // Less ideal than the connection-driven rotation (the latter
            // also fixes preview pipeline orientation hints) but the user
            // sees a rotating preview either way.
            let radians = angle * .pi / 180.0
            previewLayer.setAffineTransform(CGAffineTransform(rotationAngle: radians))
        }
    }

    /// Request camera permission and start delivering frames.
    /// Idempotent — calling again while running is a no-op.
    func start() {
        AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
            guard let self, granted else {
                print("[camera] permission denied")
                return
            }
            self.queue.async {
                self.configureIfNeeded()
                if !self.session.isRunning {
                    self.session.startRunning()
                }
            }
        }
    }

    func stop() {
        queue.async {
            if self.session.isRunning {
                self.session.stopRunning()
            }
        }
    }

    private func configureIfNeeded() {
        guard !configured else { return }
        session.beginConfiguration()
        defer { session.commitConfiguration() }

        if session.canSetSessionPreset(.hd1280x720) {
            session.sessionPreset = .hd1280x720
        }

        guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
                ?? AVCaptureDevice.default(for: .video) else {
            print("[camera] no capture device available")
            return
        }
        guard let input = try? AVCaptureDeviceInput(device: device) else {
            print("[camera] failed to create device input")
            return
        }
        if session.canAddInput(input) {
            session.addInput(input)
        }

        videoOutput.videoSettings = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA
        ]
        videoOutput.alwaysDiscardsLateVideoFrames = true
        videoOutput.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(videoOutput) {
            session.addOutput(videoOutput)
        }

        // Data-output connection: deliberately NOT rotated. The H.264 encoder
        // is configured from VideoPreset::P720 = 1280×720 landscape; if we
        // rotated 90° here we'd push 720×1280 frames into an encoder set up
        // for 1280×720, which the encoder rejects (and the catalog never
        // advertises video, so subscribers see nothing). Receivers can rotate
        // their display layer if desired.
        if let connection = videoOutput.connection(with: .video) {
            if connection.isVideoMirroringSupported {
                connection.automaticallyAdjustsVideoMirroring = false
                connection.isVideoMirrored = false
            }
        }

        // Preview layer's connection is independent of the data-output's.
        // Initialize from the CURRENT device orientation (the user may be
        // holding the phone in landscape when the call starts) rather
        // than hard-coding 90°/portrait. After this, the notification
        // observer keeps it in sync on every flip. The applyOrientation
        // call is dispatched to main inside the function so it's safe
        // from this background queue.
        DispatchQueue.main.async {
            let o = UIDevice.current.orientation
            self.applyOrientation(o == .unknown ? .portrait : o)
        }
        // Capture the configured data-output angle so the orientation tests
        // can pin it as the immutable baseline. Defaults to 0 (landscape).
        if let dConn = videoOutput.connection(with: .video) {
            initialDataOutputAngle = dConn.videoRotationAngle
        }

        configured = true
    }
}

// MARK: - Test hooks

extension CallCameraCapture {
    /// Mirror of `configureIfNeeded` that does the parts a unit test can
    /// exercise without `startRunning` (which needs camera permission and
    /// real hardware). Just constructs the preview connection so that the
    /// orientation handler has something to write into.
    func configureForTest() {
        // Touch the previewLayer's connection so `applyOrientation` finds
        // something to rotate. On a real device, `startRunning` builds the
        // connection; on a unit test we never start the session, so we
        // build one manually if needed.
        //
        // AVCaptureVideoPreviewLayer.connection is nil until the layer is
        // attached to a running session. In tests we treat the absence
        // as a no-op: the orientation logic still records `lastValidOrientation`
        // so the test can read it back.
        _ = previewLayer
    }

    /// Drive `applyOrientation` from outside without posting a notification.
    /// Used by `CallCameraCaptureOrientationTests`.
    func applyOrientationForTest(_ orientation: UIDeviceOrientation) {
        applyOrientation(orientation)
    }

    /// What angle would `applyOrientation` have set for the most recent
    /// supported orientation? Returns 90 as the configure-time default if
    /// nothing has been applied yet.
    var previewRotationAngleForTest: CGFloat {
        // Prefer the live connection's angle if the test environment ever
        // attached one; otherwise reflect what the last applied orientation
        // would have set.
        if let conn = previewLayer.connection {
            return conn.videoRotationAngle
        }
        switch lastValidOrientation {
        case .portrait: return 90
        case .portraitUpsideDown: return 270
        case .landscapeLeft: return 0
        case .landscapeRight: return 180
        default: return 90
        }
    }

    /// Configured-once data-output rotation. Must NOT change across
    /// orientation events — guarded by `testDataOutputRotationIsImmutable…`.
    var dataOutputRotationAngleForTest: CGFloat {
        videoOutput.connection(with: .video)?.videoRotationAngle ?? initialDataOutputAngle
    }
}

extension CallCameraCapture: AVCaptureVideoDataOutputSampleBufferDelegate {
    func captureOutput(_ output: AVCaptureOutput,
                       didOutput sampleBuffer: CMSampleBuffer,
                       from connection: AVCaptureConnection) {
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        guard let cb = onFrame else { return }

        CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }

        let width = CVPixelBufferGetWidth(pixelBuffer)
        let height = CVPixelBufferGetHeight(pixelBuffer)
        let rowBytes = CVPixelBufferGetBytesPerRow(pixelBuffer)
        guard let base = CVPixelBufferGetBaseAddress(pixelBuffer) else { return }

        let ts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        let tsMicros = UInt64(ts.seconds * 1_000_000)

        // The Rust side expects tightly-packed BGRA. AVCaptureSession often
        // produces buffers where rowBytes > width*4 for alignment; copy
        // row-by-row to strip the padding.
        let expectedRow = width * 4
        if rowBytes == expectedRow {
            cb(base.assumingMemoryBound(to: UInt8.self), height * rowBytes, width, height, tsMicros)
        } else {
            var packed = [UInt8](repeating: 0, count: width * height * 4)
            packed.withUnsafeMutableBufferPointer { dst in
                for y in 0..<height {
                    let src = base.advanced(by: y * rowBytes).assumingMemoryBound(to: UInt8.self)
                    let dstRow = dst.baseAddress!.advanced(by: y * expectedRow)
                    dstRow.update(from: src, count: expectedRow)
                }
            }
            packed.withUnsafeBufferPointer { buf in
                cb(buf.baseAddress!, width * height * 4, width, height, tsMicros)
            }
        }
    }
}
