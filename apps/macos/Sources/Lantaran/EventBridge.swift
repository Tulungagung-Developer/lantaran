import Foundation
import LantaranCore

/// Receives event batches on the engine's thread and hands them to the
/// model on the main thread, in order.
final class EventBridge: EventListener, @unchecked Sendable {
    private weak var model: AppModel?

    init(model: AppModel) {
        self.model = model
    }

    func onEvents(events: [FlowEventRecord], dropped: UInt64) {
        // DispatchQueue keeps batches in order; separate Tasks would not.
        DispatchQueue.main.async { [weak model] in
            MainActor.assumeIsolated {
                model?.apply(events, dropped: dropped)
            }
        }
    }
}
