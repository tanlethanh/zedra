use gpui::*;

#[derive(Clone, Debug)]
pub struct SheetDemoState {
    pub title: SharedString,
    pub subtitle: SharedString,
    pub mock_code: Vec<SharedString>,
    pub launch_count: u32,
}

impl SheetDemoState {
    pub fn new(_cx: &mut App) -> Self {
        Self::default()
    }

    pub fn mark_launched(
        &mut self,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
    ) {
        self.title = title.into();
        self.subtitle = subtitle.into();
        self.launch_count = self.launch_count.saturating_add(1);
    }
}

impl Default for SheetDemoState {
    fn default() -> Self {
        Self {
            title: "Custom Sheet Canvas".into(),
            subtitle: "Shared GPUI state rendered inside a reusable native sheet host.".into(),
            mock_code: vec![
                "let session = DeveloperSession.current".into(),
                "sheet.present(.custom) {".into(),
                "    NativeSheet(".into(),
                "        detents: [.medium, .large],".into(),
                "        grabber: true".into(),
                "    )".into(),
                "}".into(),
            ],
            launch_count: 0,
        }
    }
}
