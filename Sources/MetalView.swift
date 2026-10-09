import UIKit

final class MetalView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    override init(frame: CGRect) {
        super.init(frame: frame)
        guard let layer = layer as? CAMetalLayer else { return }
        layer.presentsWithTransaction = false
        layer.framebufferOnly = true
        contentScaleFactor = UIScreen.main.nativeScale
        isMultipleTouchEnabled = true
    }

    required init?(coder: NSCoder) {
        fatalError("MetalView is created in code only")
    }
}
