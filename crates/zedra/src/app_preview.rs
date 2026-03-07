// Standalone preview app for GPU rendering stress-testing.
//
// Replicates the full connected-session UI (DrawerHost + WorkspaceContent +
// WorkspaceDrawer + TerminalView) with mock data, bypassing remote session
// requirements. Exercises the same rendering workload that triggers OOM
// on Mali UMA GPUs.
//
// Usage: set PREVIEW_MODE = true in android/app.rs to launch in preview mode.

use gpui::*;

use crate::editor::code_editor::EditorView;
use crate::mgpui::DrawerHost;
use crate::theme;
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use crate::workspace_view::{WorkspaceContent, WorkspaceContentEvent};
use zedra_terminal::view::TerminalView;

pub struct PreviewApp {
    drawer_host: Entity<DrawerHost>,
    _workspace_drawer: Entity<WorkspaceDrawer>,
    render_count: u64,
    _subscriptions: Vec<Subscription>,
}

impl PreviewApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        zedra_terminal::load_terminal_font(window);

        let mut subscriptions = Vec::new();

        // --- EditorView with mock Rust source ---
        let content = generate_mock_rust_source(500);
        let editor = cx.new(|cx| EditorView::new(content, cx));

        // --- WorkspaceContent (header + swappable main view) ---
        let workspace_content = cx.new(|cx| WorkspaceContent::new(editor.into(), "preview.rs", cx));

        // --- DrawerHost wrapping WorkspaceContent ---
        let drawer_host = cx.new(|cx| DrawerHost::new(workspace_content.clone().into(), cx));

        // Toggle drawer when ≡ button tapped
        let drawer_host_for_toggle = drawer_host.clone();
        let sub = cx.subscribe_in(
            &workspace_content,
            window,
            move |_this: &mut Self, _emitter, event: &WorkspaceContentEvent, _window, cx| {
                match event {
                    WorkspaceContentEvent::ToggleDrawer => {
                        if drawer_host_for_toggle.read(cx).is_open() {
                            drawer_host_for_toggle.update(cx, |host, cx| host.close(cx));
                        } else {
                            drawer_host_for_toggle.update(cx, |host, cx| host.open(cx));
                        }
                    }
                    WorkspaceContentEvent::OpenQuickAction => {
                        // No-op in preview mode
                    }
                }
            },
        );
        subscriptions.push(sub);

        // --- WorkspaceDrawer ---
        let workspace_drawer = cx.new(|cx| WorkspaceDrawer::new(cx));
        drawer_host.update(cx, |host, _cx| {
            host.set_drawer(workspace_drawer.clone().into());
        });

        // Handle WorkspaceDrawer events (simplified for preview — no remote session)
        let drawer_host_for_sub = drawer_host.clone();
        let workspace_content_for_sub = workspace_content.clone();
        let sub = cx.subscribe_in(
            &workspace_drawer,
            window,
            move |_this: &mut PreviewApp,
                  _emitter: &Entity<WorkspaceDrawer>,
                  event: &WorkspaceDrawerEvent,
                  window: &mut Window,
                  cx: &mut Context<PreviewApp>| {
                match event {
                    WorkspaceDrawerEvent::CloseRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    WorkspaceDrawerEvent::DisconnectRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    WorkspaceDrawerEvent::NewTerminalRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        let terminal = create_mock_terminal(window, cx);
                        workspace_content_for_sub.update(cx, |content, cx| {
                            content.set_main_view(terminal.into(), "Terminal", cx);
                        });
                        cx.notify();
                    }
                    WorkspaceDrawerEvent::FileSelected(path) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        let filename = path.rsplit('/').next().unwrap_or(path).to_string();
                        let content = generate_mock_rust_source(300);
                        let editor = cx.new(|cx| EditorView::new(content, cx));
                        workspace_content_for_sub.update(cx, |wc, cx| {
                            wc.set_main_view(editor.into(), filename, cx);
                        });
                    }
                    WorkspaceDrawerEvent::GitFileSelected(_) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    WorkspaceDrawerEvent::TerminalSelected(_) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                }
            },
        );
        subscriptions.push(sub);

        Self {
            drawer_host,
            _workspace_drawer: workspace_drawer,
            render_count: 0,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for PreviewApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count += 1;
        if self.render_count % 60 == 1 {
            log::warn!("PreviewApp::render #{}", self.render_count);
        }

        div()
            .size_full()
            .font_family(zedra_terminal::TERMINAL_FONT_FAMILY)
            .child(
                div()
                    .size_full()
                    .bg(rgb(theme::BG_PRIMARY))
                    .flex()
                    .flex_col()
                    .child(div().flex_1().child(self.drawer_host.clone())),
            )
    }
}

/// Create a mock terminal pre-filled with realistic output.
fn create_mock_terminal(window: &mut Window, cx: &mut Context<PreviewApp>) -> Entity<TerminalView> {
    let viewport = window.viewport_size();
    let line_height = px(16.0);
    let cell_width = px(9.0);

    let columns = ((viewport.width / cell_width).floor() as usize)
        .saturating_sub(1)
        .clamp(20, 200);
    let rows = 24;

    cx.new(|cx| {
        let mut view = TerminalView::new(columns, rows, cell_width, line_height, cx);
        view.set_connected(true);
        view.set_status("Preview Terminal".to_string());
        view.advance_bytes(generate_mock_terminal_output().as_bytes());
        view
    })
}

/// Generate mock terminal output that looks like a realistic session.
fn generate_mock_terminal_output() -> String {
    let mut out = String::new();
    out.push_str("\x1b[1;32muser@host\x1b[0m:\x1b[1;34m~/projects/zedra\x1b[0m$ ls -la\r\n");
    out.push_str("total 128\r\n");
    out.push_str("drwxr-xr-x  14 user staff  448 Feb 23 10:00 .\r\n");
    out.push_str("drwxr-xr-x   8 user staff  256 Feb 22 09:15 ..\r\n");
    out.push_str("-rw-r--r--   1 user staff 8234 Feb 23 09:45 Cargo.toml\r\n");
    out.push_str("-rw-r--r--   1 user staff 4521 Feb 23 09:30 CLAUDE.md\r\n");
    out.push_str("drwxr-xr-x   6 user staff  192 Feb 23 10:00 android\r\n");
    out.push_str("drwxr-xr-x   8 user staff  256 Feb 23 09:50 crates\r\n");
    out.push_str("drwxr-xr-x   5 user staff  160 Feb 22 15:00 docs\r\n");
    out.push_str("drwxr-xr-x   3 user staff   96 Feb 21 11:00 packages\r\n");
    out.push_str("drwxr-xr-x   8 user staff  256 Feb 23 09:00 scripts\r\n");
    out.push_str("drwxr-xr-x   4 user staff  128 Feb 22 14:00 vendor\r\n\r\n");
    out.push_str("\x1b[1;32muser@host\x1b[0m:\x1b[1;34m~/projects/zedra\x1b[0m$ cargo build --release 2>&1 | tail -20\r\n");
    for i in 0..15 {
        out.push_str(&format!(
            "   \x1b[1;32mCompiling\x1b[0m crate-{} v0.1.{}\r\n",
            i, i
        ));
    }
    out.push_str("   \x1b[1;32mCompiling\x1b[0m zedra v0.1.0\r\n");
    out.push_str("    \x1b[1;32mFinished\x1b[0m release [optimized] target(s) in 42.3s\r\n\r\n");
    out.push_str("\x1b[1;32muser@host\x1b[0m:\x1b[1;34m~/projects/zedra\x1b[0m$ ");
    out
}

/// Generate a large Rust source file that exercises many syntax constructs.
fn generate_mock_rust_source(target_lines: usize) -> String {
    let mut out = String::with_capacity(target_lines * 60);

    out.push_str("// Auto-generated preview file for GPU stress-testing\n");
    out.push_str("// This exercises syntax highlighting and rendering batches.\n\n");
    out.push_str("use std::collections::HashMap;\n");
    out.push_str("use std::sync::{Arc, Mutex};\n");
    out.push_str("use std::io::{self, Read, Write};\n\n");

    for i in 0..target_lines / 30 {
        out.push_str(&format!(
            "/// Documentation comment for struct Widget{i}.\n\
             #[derive(Debug, Clone)]\n\
             pub struct Widget{i}<'a, T: Clone + Send + 'static> {{\n\
             \x20   pub id: u64,\n\
             \x20   pub name: String,\n\
             \x20   pub label: &'a str,\n\
             \x20   pub value: Option<T>,\n\
             \x20   pub children: Vec<Box<Widget{i}<'a, T>>>,\n\
             \x20   pub metadata: HashMap<String, serde_json::Value>,\n\
             \x20   counter: Arc<Mutex<usize>>,\n\
             }}\n\n"
        ));

        out.push_str(&format!(
            "impl<'a, T: Clone + Send + 'static> Widget{i}<'a, T> {{\n\
             \x20   pub fn new(name: impl Into<String>, label: &'a str) -> Self {{\n\
             \x20       Self {{\n\
             \x20           id: {id},\n\
             \x20           name: name.into(),\n\
             \x20           label,\n\
             \x20           value: None,\n\
             \x20           children: Vec::new(),\n\
             \x20           metadata: HashMap::new(),\n\
             \x20           counter: Arc::new(Mutex::new(0)),\n\
             \x20       }}\n\
             \x20   }}\n\n\
             \x20   pub fn with_value(mut self, value: T) -> Self {{\n\
             \x20       self.value = Some(value);\n\
             \x20       self\n\
             \x20   }}\n\
             }}\n\n",
            id = 1000 + i * 7,
        ));
    }

    out.push_str(
        "#[derive(Debug, Clone, PartialEq)]\n\
         pub enum Command {\n\
         \x20   Quit,\n\
         \x20   Reload { force: bool },\n\
         \x20   Navigate(String),\n\
         }\n\n",
    );

    out.push_str(
        "pub fn process_commands(commands: &[Command]) -> io::Result<Vec<String>> {\n\
         \x20   let mut results = Vec::new();\n\
         \x20   for cmd in commands {\n\
         \x20       let output = match cmd {\n\
         \x20           Command::Quit => \"quit\".to_string(),\n\
         \x20           Command::Reload { force } => format!(\"reload force={}\", force),\n\
         \x20           Command::Navigate(url) => format!(\"navigate: {}\", url),\n\
         \x20       };\n\
         \x20       results.push(output);\n\
         \x20   }\n\
         \x20   Ok(results)\n\
         }\n\n",
    );

    let current_lines = out.lines().count();
    let remaining = target_lines.saturating_sub(current_lines);
    let funcs_needed = remaining / 12 + 1;

    for i in 0..funcs_needed {
        out.push_str(&format!(
            "fn compute_hash_{i}(input: &[u8], seed: u64) -> u64 {{\n\
             \x20   let mut hash = seed;\n\
             \x20   for &byte in input {{\n\
             \x20       hash = hash.wrapping_mul(0x100000001b3).wrapping_add(byte as u64);\n\
             \x20   }}\n\
             \x20   hash\n\
             }}\n\n"
        ));
    }

    out
}
