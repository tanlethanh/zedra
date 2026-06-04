import ActivityKit
import SwiftUI
import WidgetKit

@available(iOS 16.1, *)
struct ZedraTaskLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ZedraTaskActivityAttributes.self) { context in
            // Lock screen / banner. MVP keeps it minimal and reuses the glyphs.
            HStack(spacing: 10) {
                ZedraGlyph()
                Text(headline(context.state))
                    .font(.subheadline.weight(.medium))
                Spacer()
                TrailingGlyph(state: context.state)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .activityBackgroundTint(Color.black.opacity(0.85))
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { ZedraGlyph() }
                DynamicIslandExpandedRegion(.trailing) { TrailingGlyph(state: context.state) }
                DynamicIslandExpandedRegion(.bottom) {
                    Text(headline(context.state))
                        .font(.subheadline.weight(.medium))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } compactLeading: {
                ZedraGlyph()
            } compactTrailing: {
                TrailingGlyph(state: context.state)
            } minimal: {
                TrailingGlyph(state: context.state)
            }
        }
    }

    private func headline(_ state: ZedraTaskActivityAttributes.ContentState) -> String {
        switch state.trailing {
        case .needsAction: return "Waiting for you"
        case .done: return "All done · \(state.done)/\(state.total)"
        case .loading: return "\(state.done)/\(state.total) tasks done"
        }
    }
}

/// Brand mark. Template-rendered Zedra vector from the widget's own asset catalog
/// (the extension is a separate bundle and cannot read the app's catalog).
@available(iOS 16.1, *)
private struct ZedraGlyph: View {
    var body: some View {
        Image("Zedra")
            .renderingMode(.template)
            .resizable()
            .scaledToFit()
            .foregroundStyle(.white)
            .frame(width: 22, height: 22)
    }
}

@available(iOS 16.1, *)
private struct TrailingGlyph: View {
    let state: ZedraTaskActivityAttributes.ContentState

    // One shared circular box so all three states read at the same diameter.
    private let side: CGFloat = 20

    var body: some View {
        switch state.trailing {
        case .loading:
            Text("\(state.done)/\(state.total)")
                .font(.system(size: 13, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .foregroundStyle(.white)
        case .needsAction:
            Image(systemName: "exclamationmark.circle.fill")
                .resizable()
                .symbolRenderingMode(.palette)
                .foregroundStyle(.white, .orange)
                .frame(width: side, height: side)
        case .done:
            Image(systemName: "checkmark.circle.fill")
                .resizable()
                .symbolRenderingMode(.palette)
                .foregroundStyle(.white, .green)
                .frame(width: side, height: side)
        }
    }
}
