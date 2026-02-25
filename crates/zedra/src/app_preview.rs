// Standalone preview app for GPU rendering stress-testing.
//
// Replicates the full connected-session UI (DrawerHost + EditorContent +
// AppDrawer + TerminalView) with mock data, bypassing remote session
// requirements. Exercises the same rendering workload that triggers OOM
// on Mali UMA GPUs.
//
// Usage: set PREVIEW_MODE = true in android/app.rs to launch in preview mode.

use gpui::*;

use crate::app::{EditorContent, EditorContentEvent};
use crate::app_drawer::{AppDrawer, AppDrawerEvent};
use crate::editor::code_editor::EditorView;
use crate::mgpui::{DrawerHost, HeaderConfig, StackNavigator};
use crate::theme;
use zedra_terminal::view::TerminalView;

pub struct PreviewApp {
    drawer_host: Entity<DrawerHost>,
    _editor_stack: Entity<StackNavigator>,
    _app_drawer: Entity<AppDrawer>,
    render_count: u64,
    _subscriptions: Vec<Subscription>,
}

impl PreviewApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        zedra_terminal::load_terminal_font(window);

        let mut subscriptions = Vec::new();

        // --- Editor stack with mock Rust source ---
        let editor_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(
                HeaderConfig {
                    show_header: false,
                    ..Default::default()
                },
                cx,
            );
            let content = generate_mock_rust_source(500);
            let editor = cx.new(|cx| EditorView::new(content, cx));
            stack.push(editor.into(), "preview.rs", cx);
            stack
        });

        // --- EditorContent (header + stack) ---
        let editor_content = cx.new(|cx| EditorContent::new(editor_stack.clone(), cx));

        // --- DrawerHost wrapping EditorContent ---
        let drawer_host = cx.new(|cx| DrawerHost::new(editor_content.clone().into(), cx));

        // Toggle drawer when logo button tapped
        let drawer_host_for_toggle = drawer_host.clone();
        let sub = cx.subscribe_in(
            &editor_content,
            window,
            move |_this: &mut Self, _emitter, event: &EditorContentEvent, _window, cx| match event {
                EditorContentEvent::ToggleDrawer => {
                    if drawer_host_for_toggle.read(cx).is_open() {
                        drawer_host_for_toggle.update(cx, |host, cx| host.close(cx));
                    } else {
                        drawer_host_for_toggle.update(cx, |host, cx| host.open(cx));
                    }
                }
            },
        );
        subscriptions.push(sub);

        // --- AppDrawer ---
        let app_drawer = cx.new(|cx| AppDrawer::new(cx));
        drawer_host.update(cx, |host, _cx| {
            host.set_drawer(app_drawer.clone().into());
        });

        // Handle AppDrawer events (simplified for preview — no remote session)
        let drawer_host_for_sub = drawer_host.clone();
        let editor_stack_for_sub = editor_stack.clone();
        let sub = cx.subscribe_in(
            &app_drawer,
            window,
            move |_this: &mut PreviewApp,
                  _emitter: &Entity<AppDrawer>,
                  event: &AppDrawerEvent,
                  window: &mut Window,
                  cx: &mut Context<PreviewApp>| {
                match event {
                    AppDrawerEvent::CloseRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    AppDrawerEvent::DisconnectRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    AppDrawerEvent::NewTerminalRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        let terminal = create_mock_terminal(window, cx);
                        editor_stack_for_sub.update(cx, |stack, cx| {
                            stack.replace(terminal.into(), "Terminal", cx);
                        });
                        cx.notify();
                    }
                    AppDrawerEvent::FileSelected(path) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        let filename = path.rsplit('/').next().unwrap_or(path).to_string();
                        let content = generate_mock_rust_source(300);
                        let editor = cx.new(|cx| EditorView::new(content, cx));
                        editor_stack_for_sub.update(cx, |stack, cx| {
                            stack.push(editor.into(), &filename, cx);
                        });
                    }
                    AppDrawerEvent::GitFileSelected(_) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    AppDrawerEvent::TerminalSelected(_) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                }
            },
        );
        subscriptions.push(sub);

        Self {
            drawer_host,
            _editor_stack: editor_stack,
            _app_drawer: app_drawer,
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

    let top_inset_px = crate::platform_bridge::status_bar_inset();
    let available_height = viewport.height - px(top_inset_px + 48.0);

    let columns = ((viewport.width / cell_width).floor() as usize)
        .saturating_sub(1)
        .clamp(20, 200);
    let rows = ((available_height / line_height).floor() as usize).clamp(5, 100);

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

/// Generate a large Rust source file that exercises many syntax constructs
/// (functions, structs, enums, traits, impls, generics, lifetimes, macros,
/// comments, strings, numbers) to produce a realistic batch count.
fn generate_mock_rust_source(target_lines: usize) -> String {
    let mut out = String::with_capacity(target_lines * 60);

    out.push_str("// Auto-generated preview file for GPU stress-testing\n");
    out.push_str("// This exercises syntax highlighting and rendering batches.\n\n");
    out.push_str("use std::collections::HashMap;\n");
    out.push_str("use std::sync::{Arc, Mutex};\n");
    out.push_str("use std::io::{self, Read, Write};\n\n");

    // Structs with fields
    for i in 0..target_lines / 30 {
        out.push_str(&format!(
            "/// Documentation comment for struct Widget{i}.\n\
             /// It has multiple fields of different types.\n\
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

        // Impl block with methods
        out.push_str(&format!(
            "impl<'a, T: Clone + Send + 'static> Widget{i}<'a, T> {{\n\
             \x20   /// Create a new widget with the given name.\n\
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
             \x20   }}\n\n\
             \x20   pub fn add_child(&mut self, child: Widget{i}<'a, T>) {{\n\
             \x20       self.children.push(Box::new(child));\n\
             \x20   }}\n\n\
             \x20   pub fn count(&self) -> usize {{\n\
             \x20       let guard = self.counter.lock().unwrap();\n\
             \x20       *guard\n\
             \x20   }}\n\n\
             \x20   /// Process all children recursively.\n\
             \x20   pub fn process<F>(&self, depth: usize, callback: &F)\n\
             \x20   where\n\
             \x20       F: Fn(&Widget{i}<'a, T>, usize),\n\
             \x20   {{\n\
             \x20       callback(self, depth);\n\
             \x20       for child in &self.children {{\n\
             \x20           child.process(depth + 1, callback);\n\
             \x20       }}\n\
             \x20   }}\n\
             }}\n\n",
            id = 1000 + i * 7,
        ));
    }

    // Enum with variants
    out.push_str(
        "#[derive(Debug, Clone, PartialEq)]\n\
         pub enum Command {\n\
         \x20   Quit,\n\
         \x20   Reload { force: bool },\n\
         \x20   Navigate(String),\n\
         \x20   Execute { name: String, args: Vec<String> },\n\
         \x20   Batch(Vec<Command>),\n\
         }\n\n",
    );

    // Trait definition
    out.push_str(
        "pub trait Renderer {\n\
         \x20   type Output;\n\
         \x20   type Error: std::fmt::Display;\n\n\
         \x20   fn render(&mut self, scene: &Scene) -> Result<Self::Output, Self::Error>;\n\
         \x20   fn resize(&mut self, width: u32, height: u32);\n\
         \x20   fn gpu_name(&self) -> &str;\n\
         }\n\n",
    );

    // Large function with match, loops, string formatting
    out.push_str(
        "pub fn process_commands(commands: &[Command]) -> io::Result<Vec<String>> {\n\
         \x20   let mut results = Vec::with_capacity(commands.len());\n\
         \x20   let mut total = 0u64;\n\n\
         \x20   for (index, cmd) in commands.iter().enumerate() {\n\
         \x20       let output = match cmd {\n\
         \x20           Command::Quit => {\n\
         \x20               log::info!(\"Quit at index {}\", index);\n\
         \x20               \"quit\".to_string()\n\
         \x20           }\n\
         \x20           Command::Reload { force } => {\n\
         \x20               if *force {\n\
         \x20                   format!(\"force-reload #{}\", index)\n\
         \x20               } else {\n\
         \x20                   format!(\"reload #{}\", index)\n\
         \x20               }\n\
         \x20           }\n\
         \x20           Command::Navigate(url) => {\n\
         \x20               format!(\"navigate to: {}\", url)\n\
         \x20           }\n\
         \x20           Command::Execute { name, args } => {\n\
         \x20               let joined = args.join(\" \");\n\
         \x20               format!(\"exec {} {}\", name, joined)\n\
         \x20           }\n\
         \x20           Command::Batch(sub) => {\n\
         \x20               let sub_results = process_commands(sub)?;\n\
         \x20               format!(\"batch[{}]\", sub_results.len())\n\
         \x20           }\n\
         \x20       };\n\
         \x20       total += output.len() as u64;\n\
         \x20       results.push(output);\n\
         \x20   }\n\n\
         \x20   log::info!(\"Processed {} commands, {} bytes total\", results.len(), total);\n\
         \x20   Ok(results)\n\
         }\n\n",
    );

    // Fill remaining lines with smaller functions
    let current_lines = out.lines().count();
    let remaining = target_lines.saturating_sub(current_lines);
    let funcs_needed = remaining / 12 + 1;

    for i in 0..funcs_needed {
        out.push_str(&format!(
            "/// Compute the hash for bucket {i}.\n\
             fn compute_hash_{i}(input: &[u8], seed: u64) -> u64 {{\n\
             \x20   let mut hash = seed ^ 0x{seed:016x};\n\
             \x20   for &byte in input {{\n\
             \x20       hash = hash.wrapping_mul(0x100000001b3).wrapping_add(byte as u64);\n\
             \x20   }}\n\
             \x20   // Finalize with avalanche mixing\n\
             \x20   hash ^= hash >> 33;\n\
             \x20   hash = hash.wrapping_mul(0xff51afd7ed558ccd);\n\
             \x20   hash ^= hash >> 33;\n\
             \x20   hash\n\
             }}\n\n",
            seed = 0xcbf29ce484222325u64.wrapping_add(i as u64 * 0x100),
        ));
    }

    // Final test module
    out.push_str(
        "#[cfg(test)]\n\
         mod tests {\n\
         \x20   use super::*;\n\n\
         \x20   #[test]\n\
         \x20   fn test_process_commands() {\n\
         \x20       let cmds = vec![\n\
         \x20           Command::Navigate(\"https://example.com\".into()),\n\
         \x20           Command::Reload { force: true },\n\
         \x20           Command::Execute {\n\
         \x20               name: \"ls\".into(),\n\
         \x20               args: vec![\"-la\".into(), \"/tmp\".into()],\n\
         \x20           },\n\
         \x20           Command::Batch(vec![Command::Quit]),\n\
         \x20       ];\n\
         \x20       let results = process_commands(&cmds).unwrap();\n\
         \x20       assert_eq!(results.len(), 4);\n\
         \x20   }\n\
         }\n",
    );

    out
}
