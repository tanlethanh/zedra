// Session list: shows saved/recent connections and allows connecting.
//
// This is the home screen for the Terminal tab — displays saved hosts
// from QR pairing and manual connections.

use gpui::prelude::FluentBuilder;
use gpui::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Session {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub last_connected: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionSelected(pub Session);

#[derive(Clone, Debug)]
pub struct NewSessionRequested;

// ---------------------------------------------------------------------------
// SessionList view
// ---------------------------------------------------------------------------

pub struct SessionList {
    sessions: Vec<Session>,
    focus_handle: FocusHandle,
}

impl EventEmitter<SessionSelected> for SessionList {}
impl EventEmitter<NewSessionRequested> for SessionList {}

impl SessionList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            sessions: demo_sessions(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn add_session(&mut self, session: Session, cx: &mut Context<Self>) {
        // Deduplicate by host:port
        self.sessions
            .retain(|s| !(s.host == session.host && s.port == session.port));
        self.sessions.insert(0, session);
        cx.notify();
    }
}

impl Focusable for SessionList {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SessionList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap_2().p_4();

        for (i, session) in self.sessions.iter().enumerate() {
            let s = session.clone();
            list = list.child(
                div()
                    .id(ElementId::Name(format!("session-{}", i).into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .bg(rgb(0x282c34))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(0x3e4451))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x2c313a)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            cx.emit(SessionSelected(s.clone()));
                        }),
                    )
                    // Icon
                    .child(
                        div()
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(18.0))
                            .bg(rgb(0x61afef))
                            .text_color(rgb(0x282c34))
                            .child(">_"),
                    )
                    // Info
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child(
                                div()
                                    .text_color(rgb(0xabb2bf))
                                    .child(session.name.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x5c6370))
                                    .text_sm()
                                    .child(format!("{}:{}", session.host, session.port)),
                            ),
                    ),
            );
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            // Header
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .text_color(rgb(0x61afef))
                            .text_xl()
                            .child("Sessions"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .bg(rgb(0x98c379))
                            .rounded(px(6.0))
                            .text_color(rgb(0x282c34))
                            .text_sm()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(NewSessionRequested);
                                }),
                            )
                            .child("+ New"),
                    ),
            )
            // List
            .child(
                div()
                    .id("session-list-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(list),
            )
    }
}

fn demo_sessions() -> Vec<Session> {
    vec![
        Session {
            name: "MacBook Pro".into(),
            host: "192.168.1.100".into(),
            port: 2222,
            last_connected: Some("2 min ago".into()),
        },
        Session {
            name: "Dev Server".into(),
            host: "10.0.0.50".into(),
            port: 2222,
            last_connected: Some("Yesterday".into()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_clone_and_debug() {
        let s = Session {
            name: "test".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            last_connected: None,
        };
        let s2 = s.clone();
        assert_eq!(s2.name, "test");
        assert_eq!(format!("{:?}", s2), format!("{:?}", s));
    }

    #[test]
    fn demo_sessions_not_empty() {
        let sessions = demo_sessions();
        assert!(!sessions.is_empty());
        assert!(sessions.iter().all(|s| s.port > 0));
    }
}
