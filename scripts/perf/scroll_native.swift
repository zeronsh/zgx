// Independent AppKit/Metal presentation floor. No zgx code or timers pace frames.
import AppKit
import IOSurface
import Metal
import QuartzCore

let logLock = NSLock()
func record(_ line: String) {
  logLock.lock()
  print(line)
  fflush(stdout)
  logLock.unlock()
}

@MainActor
final class Floor: NSObject, NSApplicationDelegate {
  let width: Double
  let height: Double
  let hz: Float
  let mode: String
  let duration: Double
  let delayMS: Double
  let device = MTLCreateSystemDefaultDevice()!
  var queue: MTLCommandQueue!
  var window: NSWindow!
  var link: CADisplayLink!
  let root = CALayer()
  let metal = CAMetalLayer()
  var surfaces: [IOSurfaceRef] = []
  var textures: [MTLTexture] = []
  var frame = 0
  var launched = 0.0
  var sampleStart: Double?

  init(width: Double, height: Double, hz: Float, mode: String, duration: Double, delayMS: Double) {
    self.width = width
    self.height = height
    self.hz = hz
    self.mode = mode
    self.duration = duration
    self.delayMS = delayMS
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    queue = device.makeCommandQueue()!
    window = NSWindow(
      contentRect: NSRect(x: 50, y: 50, width: width, height: height),
      styleMask: [.titled, .closable], backing: .buffered, defer: false)
    window.title = "Independent native scroll floor (\(mode))"
    let view = window.contentView!
    view.wantsLayer = true
    view.layer = root
    root.frame = view.bounds
    root.contentsScale = window.backingScaleFactor
    metal.device = device
    metal.pixelFormat = .bgra8Unorm
    metal.framebufferOnly = true
    metal.maximumDrawableCount = 3
    metal.contentsScale = window.backingScaleFactor
    let scale = window.backingScaleFactor
    let pw = Int(Double(width) * scale)
    let ph = Int(Double(height) * scale)
    if mode == "tiles" {
      metal.frame = CGRect(x: 0, y: 0, width: 1, height: 1)
      metal.drawableSize = CGSize(width: 1, height: 1)
      metal.isOpaque = false
      metal.presentsWithTransaction = true
      for _ in 0..<3 {
        let properties: [String: Any] = [
          kIOSurfaceWidth as String: pw,
          kIOSurfaceHeight as String: ph, kIOSurfaceBytesPerElement as String: 4,
          kIOSurfaceBytesPerRow as String: pw * 4,
          kIOSurfacePixelFormat as String: UInt32(0x4247_5241),
        ]
        let surface = IOSurfaceCreate(properties as CFDictionary)!
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
          pixelFormat: .bgra8Unorm,
          width: pw, height: ph, mipmapped: false)
        descriptor.storageMode = .shared
        descriptor.usage = .renderTarget
        surfaces.append(surface)
        textures.append(device.makeTexture(descriptor: descriptor, iosurface: surface, plane: 0)!)
      }
    } else {
      metal.frame = view.bounds
      metal.drawableSize = CGSize(width: pw, height: ph)
      metal.isOpaque = true
    }
    root.addSublayer(metal)
    window.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
    record("NATIVE_CONDITION \(width) \(height) \(scale) \(hz) \(mode)")
    link = view.displayLink(target: self, selector: #selector(draw(_:)))
    link.preferredFrameRateRange = CAFrameRateRange(minimum: hz, maximum: hz, preferred: hz)
    link.add(to: .main, forMode: .common)
    launched = CACurrentMediaTime()
  }

  func clear(_ texture: MTLTexture, _ buffer: MTLCommandBuffer, transparent: Bool = false) {
    let pass = MTLRenderPassDescriptor()
    pass.colorAttachments[0].texture = texture
    pass.colorAttachments[0].loadAction = .clear
    pass.colorAttachments[0].storeAction = .store
    pass.colorAttachments[0].clearColor = MTLClearColorMake(0.08, 0.12, 0.16, transparent ? 0 : 1)
    buffer.makeRenderCommandEncoder(descriptor: pass)!.endEncoding()
  }

  @objc func draw(_ clock: CADisplayLink) {
    let started = CACurrentMediaTime()
    // Control the sample from the frame clock too: a saturated drawable
    // callback can starve a main-queue dispatch timer.
    if sampleStart == nil && started - launched >= 3 {
      sampleStart = started
      record("NATIVE_BEGIN \(started)")
    }
    if let begin = sampleStart, started >= begin + duration {
      link.invalidate()
      record("NATIVE_END \(started)")
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { NSApp.terminate(nil) }
      return
    }
    frame += 1
    let n = frame
    record("NATIVE_WAKE \(n) \(started) \(clock.timestamp) \(clock.targetTimestamp)")
    let acquisition = CACurrentMediaTime()
    guard let drawable = metal.nextDrawable(), let buffer = queue.makeCommandBuffer() else {
      record("NATIVE_UNAVAILABLE \(n)")
      return
    }
    record("NATIVE_ACQUIRE \(n) \((CACurrentMediaTime()-acquisition)*1000)")
    if mode == "tiles" { clear(textures[n % 3], buffer) }
    clear(drawable.texture, buffer, transparent: mode == "tiles")
    drawable.addPresentedHandler { shown in
      record("NATIVE_PRESENT \(n) \(CACurrentMediaTime()) \(shown.presentedTime) \(started)")
    }
    buffer.addCompletedHandler { done in
      record("NATIVE_GPU \(n) \((done.gpuEndTime-done.gpuStartTime)*1000)")
    }
    // Diagnostic headroom probe: delay after encoding, before handover.
    if delayMS > 0 { Thread.sleep(forTimeInterval: delayMS / 1000) }
    if mode == "tiles" {
      buffer.commit()
      buffer.waitUntilCompleted()
      let handover = CACurrentMediaTime()
      CATransaction.begin()
      CATransaction.setDisableActions(true)
      root.contents = surfaces[n % 3]
      drawable.present()
      CATransaction.commit()
      CATransaction.flush()
      record("NATIVE_HANDOVER \(n) \(handover) \(CACurrentMediaTime())")
    } else {
      buffer.present(drawable)
      let handover = CACurrentMediaTime()
      buffer.commit()
      record("NATIVE_HANDOVER \(n) \(handover) \(CACurrentMediaTime())")
    }
    record("NATIVE_WORK \(n) \((CACurrentMediaTime()-started)*1000)")
  }
}

let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 5 || args.count == 6, let width = Double(args[0]), let height = Double(args[1]),
  let hz = Float(args[2]), let duration = Double(args[4]), ["drawables", "tiles"].contains(args[3])
else {
  fatalError("usage: scroll-native WIDTH HEIGHT HZ drawables|tiles SECONDS")
}
MainActor.assumeIsolated {
  let app = NSApplication.shared
  app.setActivationPolicy(.regular)
  let delegate = Floor(
    width: width, height: height, hz: hz, mode: args[3], duration: duration,
    delayMS: args.count == 6 ? Double(args[5])! : 0)
  app.delegate = delegate
  app.run()
}
