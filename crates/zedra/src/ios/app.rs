/// iOS App State — main thread application logic
///
/// Processes commands from the queue and manages session lifecycle.
/// This is the iOS equivalent of android/app.rs, but without GPUI —
/// the UI is driven by SwiftUI, and this module manages the Rust backend.
use anyhow::Result;
use std::sync::Mutex;

use super::command_queue::IosCommand;

/// Terminal output buffer that Swift polls each frame
static TERMINAL_OUTPUT: Mutex<String> = Mutex::new(String::new());

/// Connection status for Swift to read
static CONNECTION_STATUS: Mutex<ConnectionStatus> = Mutex::new(ConnectionStatus::Disconnected);

/// Transport info for Swift to read
static TRANSPORT_INFO: Mutex<String> = Mutex::new(String::new());

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

/// iOS app state — must only be accessed from the main thread
pub struct IosApp {
    initialized: bool,
    screen_width: f32,
    screen_height: f32,
    scale: f32,
}

impl IosApp {
    pub fn new() -> Self {
        Self {
            initialized: false,
            screen_width: 0.0,
            screen_height: 0.0,
            scale: 1.0,
        }
    }

    pub fn process_command(&mut self, command: IosCommand) -> Result<()> {
        match command {
            IosCommand::Initialize {
                screen_width,
                screen_height,
                scale,
            } => self.handle_initialize(screen_width, screen_height, scale),
            IosCommand::ViewResized { width, height } => self.handle_resize(width, height),
            IosCommand::Touch { action, x, y } => self.handle_touch(action, x, y),
            IosCommand::TextInput { text } => self.handle_text_input(&text),
            IosCommand::KeyEvent { key_name } => self.handle_key_event(&key_name),
            IosCommand::Connect { host, port } => self.handle_connect(host, port),
            IosCommand::Disconnect => self.handle_disconnect(),
            IosCommand::PairViaQr { qr_data } => self.handle_pair_via_qr(qr_data),
            IosCommand::SendInput { text } => self.handle_send_input(&text),
            IosCommand::Resume => self.handle_resume(),
            IosCommand::Pause => self.handle_pause(),
            IosCommand::RequestFrame => self.handle_frame(),
        }
    }

    fn handle_initialize(
        &mut self,
        screen_width: f32,
        screen_height: f32,
        scale: f32,
    ) -> Result<()> {
        log::info!(
            "iOS app initialized: {}x{} @ {}x scale",
            screen_width,
            screen_height,
            scale
        );
        self.screen_width = screen_width;
        self.screen_height = screen_height;
        self.scale = scale;
        self.initialized = true;
        Ok(())
    }

    fn handle_resize(&mut self, width: f32, height: f32) -> Result<()> {
        log::info!("View resized: {}x{}", width, height);
        self.screen_width = width;
        self.screen_height = height;
        Ok(())
    }

    fn handle_touch(&mut self, action: i32, x: f32, y: f32) -> Result<()> {
        log::trace!("Touch: action={}, pos=({:.1}, {:.1})", action, x, y);
        Ok(())
    }

    fn handle_text_input(&mut self, text: &str) -> Result<()> {
        log::debug!("Text input: {}", text);
        self.send_terminal_input(text);
        Ok(())
    }

    fn handle_key_event(&mut self, key_name: &str) -> Result<()> {
        log::debug!("Key event: {}", key_name);
        let escape_seq = match key_name {
            "backspace" => "\x7f",
            "enter" => "\r",
            "tab" => "\t",
            "escape" => "\x1b",
            "up" => "\x1b[A",
            "down" => "\x1b[B",
            "right" => "\x1b[C",
            "left" => "\x1b[D",
            _ => return Ok(()),
        };
        self.send_terminal_input(escape_seq);
        Ok(())
    }

    fn handle_connect(&mut self, host: String, port: u16) -> Result<()> {
        log::info!("Connecting to {}:{}...", host, port);
        set_connection_status(ConnectionStatus::Connecting);

        let cols = ((self.screen_width / 9.0) as u16).clamp(20, 200);
        let rows = ((self.screen_height / 16.0) as u16).clamp(5, 100);

        // Parse host:port as an iroh endpoint address
        // For direct connections, we construct the address from the QR pairing flow
        // For now, log a warning that direct host:port connect is not supported in v2
        log::warn!(
            "Direct host:port connection is deprecated in v2. Use QR pairing instead. \
             Attempted: {}:{}",
            host,
            port
        );
        set_connection_status(ConnectionStatus::Error(
            "Direct host:port connect is not supported. Use QR pairing.".to_string(),
        ));
        let _ = (cols, rows); // suppress unused warning
        Ok(())
    }

    fn handle_disconnect(&mut self) -> Result<()> {
        log::info!("Disconnecting...");
        zedra_session::clear_active_session();
        set_connection_status(ConnectionStatus::Disconnected);
        Ok(())
    }

    fn handle_pair_via_qr(&mut self, qr_data: String) -> Result<()> {
        log::info!("QR pairing: {}", &qr_data[..qr_data.len().min(50)]);
        set_connection_status(ConnectionStatus::Connecting);

        let cols = ((self.screen_width / 9.0) as u16).clamp(20, 200);
        let rows = ((self.screen_height / 16.0) as u16).clamp(5, 100);

        // Parse the zedra:// URI to extract the iroh EndpointAddr
        match zedra_rpc::pairing::decode_endpoint_addr(&qr_data) {
            Ok(addr) => {
                log::info!("QR parsed: endpoint addr decoded");

                zedra_session::session_runtime().spawn(async move {
                    match zedra_session::RemoteSession::connect_with_iroh(addr).await {
                        Ok(session) => {
                            log::info!("RemoteSession: connected via iroh!");
                            match session.terminal_create(cols, rows).await {
                                Ok(term_id) => {
                                    log::info!("Remote terminal created: {}", term_id);
                                }
                                Err(e) => {
                                    log::error!("Failed to create remote terminal: {}", e);
                                    set_connection_status(ConnectionStatus::Error(e.to_string()));
                                    return;
                                }
                            }
                            zedra_session::set_active_session(session);
                            set_connection_status(ConnectionStatus::Connected);
                            zedra_session::signal_terminal_data();
                        }
                        Err(e) => {
                            log::error!("RemoteSession iroh connect failed: {}", e);
                            set_connection_status(ConnectionStatus::Error(e.to_string()));
                        }
                    }
                });
            }
            Err(e) => {
                log::error!("Failed to decode QR endpoint addr: {}", e);
                set_connection_status(ConnectionStatus::Error(e.to_string()));
            }
        }
        Ok(())
    }

    fn handle_send_input(&mut self, text: &str) -> Result<()> {
        self.send_terminal_input(text);
        Ok(())
    }

    fn send_terminal_input(&self, text: &str) {
        zedra_session::send_terminal_input(text.as_bytes().to_vec());
    }

    fn handle_resume(&mut self) -> Result<()> {
        log::info!("App resumed");
        Ok(())
    }

    fn handle_pause(&mut self) -> Result<()> {
        log::info!("App paused");
        Ok(())
    }

    fn handle_frame(&mut self) -> Result<()> {
        // Drain session callbacks
        for cb in zedra_session::drain_callbacks() {
            cb();
        }

        // Collect terminal output from active session
        if zedra_session::check_and_clear_terminal_data() {
            if let Some(session) = zedra_session::active_session() {
                if let Some(buf) = session.output_buffer_for(
                    &session.active_terminal_id().unwrap_or_default(),
                ) {
                    if let Ok(mut guard) = buf.lock() {
                        while let Some(chunk) = guard.pop_front() {
                            if let Ok(text) = String::from_utf8(chunk) {
                                append_terminal_output(&text);
                            }
                        }
                    }
                }

                // Update transport info from session state
                let info = format_session_info(&session);
                if !info.is_empty() {
                    set_transport_info(&info);
                }
            }
        }

        Ok(())
    }
}

impl Default for IosApp {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local storage for the IosApp instance
thread_local! {
    static IOS_APP: std::cell::RefCell<Option<IosApp>> = std::cell::RefCell::new(None);
}

pub fn init_ios_app() {
    IOS_APP.with(|app| {
        if app.borrow().is_none() {
            *app.borrow_mut() = Some(IosApp::new());
        }
    });
}

pub fn process_commands_from_queue() -> Result<()> {
    let commands = super::command_queue::drain_commands();

    IOS_APP.with(|app_cell| {
        let mut app_opt = app_cell.borrow_mut();

        if let Some(app) = app_opt.as_mut() {
            for command in commands {
                if let Err(e) = app.process_command(command) {
                    log::error!("Error processing command: {}", e);
                }
            }

            if let Err(e) = app.handle_frame() {
                log::error!("Error in frame tick: {}", e);
            }

            Ok(())
        } else {
            Err(anyhow::anyhow!("IosApp not initialized"))
        }
    })
}

// -- Shared state helpers --

fn set_connection_status(status: ConnectionStatus) {
    if let Ok(mut s) = CONNECTION_STATUS.lock() {
        *s = status;
    }
}

pub fn get_connection_status() -> ConnectionStatus {
    CONNECTION_STATUS
        .lock()
        .map(|s| s.clone())
        .unwrap_or(ConnectionStatus::Disconnected)
}

fn append_terminal_output(text: &str) {
    if let Ok(mut buf) = TERMINAL_OUTPUT.lock() {
        buf.push_str(text);
    }
}

pub fn take_terminal_output() -> String {
    if let Ok(mut buf) = TERMINAL_OUTPUT.lock() {
        std::mem::take(&mut *buf)
    } else {
        String::new()
    }
}

fn set_transport_info(info: &str) {
    if let Ok(mut t) = TRANSPORT_INFO.lock() {
        *t = info.to_string();
    }
}

pub fn get_transport_info() -> String {
    TRANSPORT_INFO
        .lock()
        .map(|t| t.clone())
        .unwrap_or_default()
}

fn format_session_info(session: &zedra_session::RemoteSession) -> String {
    use zedra_session::SessionState;
    let state = session.state();
    let latency = session.latency_ms();

    match state {
        SessionState::Connected { .. } => {
            let conn_type = session
                .connection_info()
                .map(|ci| {
                    if ci.is_direct {
                        "P2P"
                    } else {
                        "Relay"
                    }
                })
                .unwrap_or("Connected");

            if latency > 0 {
                format!("{} · {}ms", conn_type, latency)
            } else {
                conn_type.to_string()
            }
        }
        SessionState::Connecting => "Connecting...".to_string(),
        SessionState::Reconnecting { attempt } => format!("Reconnecting ({})", attempt),
        SessionState::Disconnected => String::new(),
        SessionState::Error(msg) => format!("Error: {}", msg),
    }
}
