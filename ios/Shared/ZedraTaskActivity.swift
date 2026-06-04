import ActivityKit
import Foundation

/// Shared between the app and the widget extension via target membership. The
/// activity type name (`ZedraTaskActivityAttributes`) is what ActivityKit routes on,
/// so this declaration must stay identical on both sides.
@available(iOS 16.1, *)
struct ZedraTaskActivityAttributes: ActivityAttributes {
    /// Aggregate state across all workspaces/sessions. Delta owns the merge and
    /// pushes the resolved view; the app only renders what arrives.
    struct ContentState: Codable, Hashable {
        enum Trailing: String, Codable, Hashable {
            case loading      // tasks running, nothing else to surface
            case needsAction  // a task is waiting on the user
            case done         // a task just finished (holds until `doneUntil`)
        }

        var trailing: Trailing
        /// Finished tasks in the aggregate.
        var done: Int
        /// Total tracked tasks across all workspaces/sessions.
        var total: Int
        /// When a `done` tick should revert/end. Drives the system countdown.
        var doneUntil: Date?

        static let idle = ContentState(trailing: .loading, done: 0, total: 0, doneUntil: nil)
    }

    /// Stable per install. The aggregate activity has no per-task identity.
    var appName: String = "Zedra"
}
