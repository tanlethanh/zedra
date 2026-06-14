import ActivityKit
import Foundation

/// App-side owner of the single aggregate Live Activity.
///
/// Tracks per-task status keyed by a stable task id (a terminal id in production) and
/// renders the aggregate `done/total` rollup. MVP drives it locally for on-device
/// testing via `zedra://la-test/<action>/<taskId>`. Delta-driven remote updates
/// (token reporting, reattach, push-to-start) are the next increment — see
/// `docs/LIVE_ACTIVITY.md`.
@MainActor
@available(iOS 16.2, *)
enum LiveActivityController {
    // Debug-only on-device scaffold. The entire local-aggregation path is excluded
    // from release builds, where Delta drives the aggregate Live Activity instead.
    #if DEBUG
    private enum TaskState: String { case running, needsInput, done }

    /// Aggregate task table. Key = task id (terminal id). Persisted so it survives app
    /// relaunch (each deeplink from Safari cold-launches the app under the debugger,
    /// and in production the activity outlives the process).
    private static let storeKey = "zedra.liveactivity.tasks"

    private static func loadTasks() -> [String: TaskState] {
        let raw = UserDefaults.standard.dictionary(forKey: storeKey) as? [String: String] ?? [:]
        return raw.compactMapValues(TaskState.init(rawValue:))
    }

    private static func saveTasks(_ tasks: [String: TaskState]) {
        UserDefaults.standard.set(tasks.mapValues(\.rawValue), forKey: storeKey)
    }

    private static var current: Activity<ZedraTaskActivityAttributes>? {
        Activity<ZedraTaskActivityAttributes>.activities.first
    }

    private static let tickWindow: TimeInterval = 20

    /// `zedra://la-test/<action>/<taskId>` — action in start|needs|done|end.
    /// `taskId` binds the action to a specific task (e.g. terminal id); omit it on
    /// `end` to clear the whole activity. Debug-only scaffold: in release Delta drives
    /// the aggregate Live Activity, so this path is never compiled in.
    static func handleTestURL(_ url: URL) -> Bool {
        guard url.host == "la-test" else { return false }
        let parts = url.pathComponents.filter { $0 != "/" }
        let action = parts.first ?? ""
        let taskId = parts.count > 1 ? parts[1] : nil
        NSLog("[LA] test deeplink action=%@ task=%@", action, taskId ?? "—")
        Task { await dispatch(action, taskId: taskId) }
        return true
    }

    private static func dispatch(_ action: String, taskId: String?) async {
        var tasks = loadTasks()
        let id = taskId ?? "default"
        switch action {
        case "start":
            tasks[id] = .running
        case "needs":
            tasks[id] = .needsInput
        case "done":
            tasks[id] = .done
        case "end":
            if let taskId {
                tasks[taskId] = nil
            } else {
                tasks.removeAll()
            }
        default:
            NSLog("[LA] unknown action=%@ (use start|needs|done|end)", action)
            return
        }
        saveTasks(tasks)

        if tasks.isEmpty {
            await endAll()
        } else {
            await render(tasks)
        }
    }

    /// Resolve the aggregate table into a content state and push it.
    private static func render(_ tasks: [String: TaskState]) async {
        let total = tasks.count
        let done = tasks.values.filter { $0 == .done }.count
        let needsAttention = tasks.values.contains(.needsInput)

        // Priority: a task waiting on the user outranks a finished tick, which outranks
        // routine progress.
        let trailing: ZedraTaskActivityAttributes.ContentState.Trailing
        if needsAttention {
            trailing = .needsAction
        } else if total > 0, done == total {
            trailing = .done
        } else {
            trailing = .loading
        }

        await ensure(.init(
            trailing: trailing,
            done: done,
            total: total,
            doneUntil: trailing == .done ? .now + tickWindow : nil
        ))
    }

    /// Start the activity if needed, otherwise update it in place.
    private static func ensure(_ state: ZedraTaskActivityAttributes.ContentState) async {
        guard ActivityAuthorizationInfo().areActivitiesEnabled else {
            NSLog("[LA] disabled in Settings — cannot start")
            return
        }
        if let activity = current {
            await activity.update(ActivityContent(state: state, staleDate: nil))
            NSLog("[LA] updated trailing=%@ done=%d total=%d",
                  state.trailing.rawValue, state.done, state.total)
            return
        }
        do {
            let activity = try Activity.request(
                attributes: ZedraTaskActivityAttributes(),
                content: ActivityContent(state: state, staleDate: nil),
                pushType: nil // local-only for MVP; .token once Delta drives it
            )
            NSLog("[LA] started activity id=%@ trailing=%@ done=%d total=%d",
                  activity.id, state.trailing.rawValue, state.done, state.total)
        } catch {
            NSLog("[LA] start failed: %@", error.localizedDescription)
        }
    }

    private static func endAll() async {
        saveTasks([:])
        let activities = Activity<ZedraTaskActivityAttributes>.activities
        NSLog("[LA] ending %d activity(ies)", activities.count)
        for activity in activities {
            await activity.end(nil, dismissalPolicy: .immediate)
        }
    }
    #endif
}
