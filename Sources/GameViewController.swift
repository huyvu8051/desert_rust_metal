import UIKit
import QuartzCore

final class GameViewController: UIViewController {
    private var metalView: MetalView!
    private var renderer: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?
    private var lastFrameTime = CACurrentMediaTime()

    private var moveTouch: UITouch?
    private var moveTouchStart: CGPoint = .zero
    private var moveTouchCurrent: CGPoint = .zero
    private var lookTouch: UITouch?
    private var lookTouchLast: CGPoint = .zero
    private var pendingLook: CGPoint = .zero

    private let joystickRadius: CGFloat = 70

    override func viewDidLoad() {
        super.viewDidLoad()

        metalView = MetalView(frame: view.bounds)
        metalView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(metalView)
        view.isMultipleTouchEnabled = true

        view.layoutIfNeeded()
        renderer = baseline_create(Unmanaged.passUnretained(metalView).toOpaque())

        let link = CADisplayLink(target: self, selector: #selector(step))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    @objc private func step() {
        guard let renderer else { return }

        let now = CACurrentMediaTime()
        let dt = Float(min(now - lastFrameTime, 0.1))
        lastFrameTime = now

        var forward: Float = 0
        var strafe: Float = 0
        if moveTouch != nil {
            let dx = moveTouchCurrent.x - moveTouchStart.x
            let dy = moveTouchCurrent.y - moveTouchStart.y
            forward = Float(-dy / joystickRadius).clamped(to: -1...1)
            strafe = Float(dx / joystickRadius).clamped(to: -1...1)
        }

        baseline_update_camera(renderer, forward, strafe, Float(pendingLook.x), Float(pendingLook.y), dt)
        pendingLook = .zero
        baseline_render_frame(renderer)
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            let point = touch.location(in: view)
            if point.x < view.bounds.midX {
                if moveTouch == nil {
                    moveTouch = touch
                    moveTouchStart = point
                    moveTouchCurrent = point
                }
            } else if lookTouch == nil {
                lookTouch = touch
                lookTouchLast = point
            }
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            let point = touch.location(in: view)
            if touch === moveTouch {
                moveTouchCurrent = point
            } else if touch === lookTouch {
                pendingLook.x += point.x - lookTouchLast.x
                pendingLook.y += point.y - lookTouchLast.y
                lookTouchLast = point
            }
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        releaseTouches(touches)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        releaseTouches(touches)
    }

    private func releaseTouches(_ touches: Set<UITouch>) {
        for touch in touches {
            if touch === moveTouch { moveTouch = nil }
            if touch === lookTouch { lookTouch = nil }
        }
    }

    deinit {
        displayLink?.invalidate()
        if let renderer { baseline_release(renderer) }
    }

    override var prefersStatusBarHidden: Bool { true }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
