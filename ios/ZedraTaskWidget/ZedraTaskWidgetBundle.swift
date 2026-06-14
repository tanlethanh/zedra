import SwiftUI
import WidgetKit

@main
struct ZedraTaskWidgetBundle: WidgetBundle {
    var body: some Widget {
        if #available(iOS 16.1, *) {
            ZedraTaskLiveActivity()
        }
    }
}
