// ProjectEditor — split-pane file explorer + code editor
//
// Left sidebar: FileExplorer (animated gesture-based drawer)
// Right content: EditorView showing the currently selected file
// Used when a remote session is active, replacing the FilePreviewList demo view.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use zedra_editor::EditorView;

const SIDEBAR_WIDTH: f32 = 260.0;

// ---------------------------------------------------------------------------
// DrawerState for gesture-based animated sidebar
// ---------------------------------------------------------------------------

struct DrawerState {
    /// Current drawer offset (0 = closed, SIDEBAR_WIDTH = fully open)
    offset: f32,
    /// Whether a drag gesture is in progress
    is_dragging: bool,
}

impl Default for DrawerState {
    fn default() -> Self {
        Self {
            offset: 0.0,
            is_dragging: false,
        }
    }
}

/// Once a gesture's axis is determined, lock it for the rest of the touch.
#[derive(Clone, Copy, PartialEq)]
enum GestureAxis {
    Undecided,
    Horizontal, // drawer swipe
    Vertical,   // content scroll — ignore in drawer handler
}

// ---------------------------------------------------------------------------
// ProjectEditor view
// ---------------------------------------------------------------------------

pub struct ProjectEditor {
    file_explorer: Entity<FileExplorer>,
    editor_view: Entity<EditorView>,
    current_file: Option<String>,
    error_message: Option<String>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
    /// Instance-owned buffer for async file content delivery
    pending_file: Arc<Mutex<Option<(String, String)>>>,
    /// Instance-owned buffer for async error delivery
    pending_error: Arc<Mutex<Option<String>>>,
    /// Shared drawer state for gesture handling
    drawer_state: Arc<Mutex<DrawerState>>,
    /// Animation start offset
    snap_from: f32,
    /// Animation target offset (None = no animation in progress)
    snap_target: Option<f32>,
    /// Incremented each snap to retrigger with_animation
    animation_id: u64,
    /// Locked gesture axis for current touch sequence
    gesture_axis: GestureAxis,
    /// Accumulated deltas to decide axis before locking
    gesture_accum: (f32, f32),
}

impl ProjectEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let file_explorer = cx.new(|cx| FileExplorer::new(cx));
        let editor_view =
            cx.new(|_cx| EditorView::new("// Open a file from the sidebar".to_string(), _cx));

        let pending_file: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));
        let pending_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let mut subscriptions = Vec::new();

        // Subscribe to FileSelected events from the explorer
        let pf = pending_file.clone();
        let pe = pending_error.clone();
        let sub = cx.subscribe(
            &file_explorer,
            move |this: &mut Self, _emitter, event: &FileSelected, cx| {
                // Empty path = close button tapped
                if event.path.is_empty() {
                    this.start_snap(0.0, cx);
                    return;
                }
                log::info!("ProjectEditor: file selected: {}", event.path);
                let path = event.path.clone();
                let filename = path.rsplit('/').next().unwrap_or(&path).to_string();

                if let Some(session) = zedra_session::active_session() {
                    let pf = pf.clone();
                    let pe = pe.clone();
                    let filename_clone = filename.clone();
                    zedra_session::session_runtime().spawn(async move {
                        match session.fs_read(&path).await {
                            Ok(content) => {
                                if let Ok(mut slot) = pf.lock() {
                                    *slot = Some((filename_clone, content));
                                }
                                zedra_session::signal_terminal_data();
                            }
                            Err(e) => {
                                log::error!("ProjectEditor: fs/read failed for {}: {}", path, e);
                                if let Ok(mut slot) = pe.lock() {
                                    *slot = Some(format!("Failed to read {}: {}", path, e));
                                }
                                zedra_session::signal_terminal_data();
                            }
                        }
                    });
                }

                // Close the sidebar after selection
                this.start_snap(0.0, cx);
            },
        );
        subscriptions.push(sub);

        Self {
            file_explorer,
            editor_view,
            current_file: None,
            error_message: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            pending_file,
            pending_error,
            drawer_state: Arc::new(Mutex::new(DrawerState::default())),
            snap_from: 0.0,
            snap_target: None,
            animation_id: 0,
            gesture_axis: GestureAxis::Undecided,
            gesture_accum: (0.0, 0.0),
        }
    }

    /// Pick up any pending file content or error from async tasks
    fn apply_pending(&mut self, cx: &mut Context<Self>) {
        if let Some((filename, content)) =
            self.pending_file.try_lock().ok().and_then(|mut s| s.take())
        {
            log::info!(
                "ProjectEditor: loading file '{}', {} bytes",
                filename,
                content.len()
            );
            self.current_file = Some(filename);
            self.error_message = None;
            self.editor_view
                .update(cx, |view, _cx| view.set_content(content));
        }

        if let Some(err) = self
            .pending_error
            .try_lock()
            .ok()
            .and_then(|mut s| s.take())
        {
            self.error_message = Some(err);
        }
    }

    /// Start a snap animation to the given target offset
    fn start_snap(&mut self, target: f32, cx: &mut Context<Self>) {
        let current = self.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
        if let Ok(mut state) = self.drawer_state.lock() {
            state.offset = target;
            state.is_dragging = false;
        }
        self.snap_from = current;
        self.snap_target = Some(target);
        self.animation_id += 1;
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        let current = self.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
        let target = if current > SIDEBAR_WIDTH / 2.0 {
            0.0
        } else {
            SIDEBAR_WIDTH
        };
        self.start_snap(target, cx);
    }
}

impl Focusable for ProjectEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ProjectEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_pending(cx);

        let filename_display = self.current_file.as_deref().unwrap_or("Select a file");

        let error_msg = self.error_message.clone();

        // Read drawer state
        let (drawer_offset, is_dragging) = self
            .drawer_state
            .lock()
            .map(|s| (s.offset, s.is_dragging))
            .unwrap_or((0.0, false));

        let is_open = drawer_offset > 0.0;
        let snap_target = self.snap_target;
        let snap_from = self.snap_from;
        let animation_id = self.animation_id;
        let animating = snap_target.is_some() && !is_dragging;

        // Content area: editor fills full width, sidebar overlays on top
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0e0c0c))
            .track_focus(&self.focus_handle)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    // Reset gesture axis for next touch
                    this.gesture_axis = GestureAxis::Undecided;
                    this.gesture_accum = (0.0, 0.0);

                    let was_dragging = this
                        .drawer_state
                        .lock()
                        .map(|s| s.is_dragging)
                        .unwrap_or(false);
                    if was_dragging {
                        let current = this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
                        let target = if current > SIDEBAR_WIDTH / 2.0 {
                            SIDEBAR_WIDTH
                        } else {
                            0.0
                        };
                        this.start_snap(target, cx);
                    }
                }),
            )
            // Editor — always full width
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        // Filename bar with sidebar toggle
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .h(px(28.0))
                            .px_2()
                            .bg(rgb(0x0e0c0c))
                            .border_b_1()
                            .border_color(rgb(0x1a1a1a))
                            .child(
                                div()
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .text_color(rgb(0x505050))
                                    .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.05)))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                                    )
                                    .text_size(px(12.0))
                                    .child(if is_open || snap_target.map_or(false, |t| t > 0.0) {
                                        "<"
                                    } else {
                                        ">"
                                    }),
                            )
                            .child(
                                div()
                                    .ml_1()
                                    .text_color(rgb(0x505050))
                                    .text_size(px(12.0))
                                    .child(filename_display.to_string()),
                            ),
                    )
                    // Error bar (shown when fs_read fails)
                    .when_some(error_msg, |el, msg| {
                        el.child(
                            div()
                                .px_2()
                                .py_1()
                                .bg(rgb(0x3d1f1f))
                                .border_b_1()
                                .border_color(rgb(0x6b2e2e))
                                .text_color(rgb(0xe06c75))
                                .text_size(px(11.0))
                                .child(msg),
                        )
                    })
                    .child(div().flex_1().child(self.editor_view.clone())),
            )
            // Left-edge swipe trigger — invisible 40px zone on top of content.
            // Sits above the editor so it captures scroll events before uniform_list.
            .when(!is_open && snap_target.is_none(), |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(40.0))
                        .on_scroll_wheel(cx.listener(
                            |this, event: &ScrollWheelEvent, _window, cx| {
                                let (dx, dy) = match event.delta {
                                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                                    ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
                                };

                                if this.gesture_axis == GestureAxis::Vertical {
                                    return;
                                }

                                if this.gesture_axis == GestureAxis::Horizontal {
                                    this.snap_target = None;
                                    if let Ok(mut state) = this.drawer_state.lock() {
                                        state.is_dragging = true;
                                        state.offset =
                                            (state.offset + dx).clamp(0.0, SIDEBAR_WIDTH);
                                    }
                                    cx.notify();
                                    return;
                                }

                                // Undecided — accumulate then lock axis
                                this.gesture_accum.0 += dx;
                                this.gesture_accum.1 += dy;
                                let (ax, ay) =
                                    (this.gesture_accum.0.abs(), this.gesture_accum.1.abs());

                                if ax + ay > 6.0 {
                                    if ax > ay {
                                        this.gesture_axis = GestureAxis::Horizontal;
                                        this.snap_target = None;
                                        if let Ok(mut state) = this.drawer_state.lock() {
                                            state.is_dragging = true;
                                            state.offset = (state.offset + this.gesture_accum.0)
                                                .clamp(0.0, SIDEBAR_WIDTH);
                                        }
                                        cx.notify();
                                    } else {
                                        this.gesture_axis = GestureAxis::Vertical;
                                    }
                                }
                            },
                        )),
                )
            })
            // Sidebar overlay: backdrop + file explorer panel
            // Present when drawer is open or animating
            .when(is_open || snap_target.is_some(), |el| {
                // Backdrop — covers full area, tappable to close, swipeable
                let backdrop = div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.start_snap(0.0, cx);
                        }),
                    )
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                        let dx = match event.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.x),
                            ScrollDelta::Lines(l) => l.x * 20.0,
                        };
                        if dx.abs() > 1.0 {
                            this.snap_target = None;
                            if let Ok(mut state) = this.drawer_state.lock() {
                                state.is_dragging = true;
                                state.offset = (state.offset + dx).clamp(0.0, SIDEBAR_WIDTH);
                            }
                            cx.notify();
                        }
                    }));

                let backdrop: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    backdrop
                        .with_animation(
                            ElementId::NamedInteger("backdrop-snap".into(), animation_id),
                            Animation::new(Duration::from_millis(250))
                                .with_easing(ease_out_quint()),
                            move |elem, delta| {
                                let o = from + (target - from) * delta;
                                let opacity = (o / SIDEBAR_WIDTH * 0.4).clamp(0.0, 0.4);
                                elem.bg(hsla(0.0, 0.0, 0.0, opacity))
                            },
                        )
                        .into_any_element()
                } else {
                    let opacity = (drawer_offset / SIDEBAR_WIDTH * 0.4).clamp(0.0, 0.4);
                    backdrop.bg(hsla(0.0, 0.0, 0.0, opacity)).into_any_element()
                };

                // Sidebar panel
                let sidebar = div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(px(SIDEBAR_WIDTH))
                    .bg(rgb(0x0e0c0c))
                    .child(self.file_explorer.clone());

                let sidebar: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    sidebar
                        .with_animation(
                            ElementId::NamedInteger("drawer-snap".into(), animation_id),
                            Animation::new(Duration::from_millis(250))
                                .with_easing(ease_out_quint()),
                            move |elem, delta| {
                                let o = from + (target - from) * delta;
                                elem.left(px(o - SIDEBAR_WIDTH))
                            },
                        )
                        .into_any_element()
                } else {
                    sidebar
                        .left(px(drawer_offset - SIDEBAR_WIDTH))
                        .into_any_element()
                };

                el.child(backdrop).child(sidebar)
            })
    }
}
