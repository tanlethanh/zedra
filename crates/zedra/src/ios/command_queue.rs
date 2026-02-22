/// Thread-safe command queue for iOS Swift → Rust Main Thread communication
///
/// Same architecture as the Android version:
///   Any Thread → Command Queue → Main Thread (via CADisplayLink at 60 FPS)
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};

/// Commands that can be sent from Swift to the Rust main thread
#[derive(Debug)]
pub enum IosCommand {
    /// Initialize the app (called once at launch)
    Initialize {
        screen_width: f32,
        screen_height: f32,
        scale: f32,
    },
    /// Surface/view was resized
    ViewResized {
        width: f32,
        height: f32,
    },
    /// Touch event (action: 0=down, 1=up, 2=move, 3=cancel)
    Touch {
        action: i32,
        x: f32,
        y: f32,
    },
    /// Text input from iOS keyboard
    TextInput {
        text: String,
    },
    /// Key event (backspace, enter, etc.)
    KeyEvent {
        key_name: String,
    },
    /// Connect to host via direct TCP
    Connect {
        host: String,
        port: u16,
    },
    /// Disconnect active session
    Disconnect,
    /// QR code scanned — initiate pairing
    PairViaQr {
        qr_data: String,
    },
    /// Send terminal input text
    SendInput {
        text: String,
    },
    /// App entered foreground
    Resume,
    /// App entered background
    Pause,
    /// Frame tick (from CADisplayLink at 60 FPS)
    RequestFrame,
}

/// Thread-safe command queue
pub struct IosCommandQueue {
    sender: Sender<IosCommand>,
    receiver: Receiver<IosCommand>,
}

impl IosCommandQueue {
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver }
    }

    /// Send a command from any thread
    pub fn send(&self, command: IosCommand) -> Result<()> {
        self.sender
            .send(command)
            .map_err(|e| anyhow::anyhow!("Failed to send command: {}", e))
    }

    /// Drain all pending commands (called from main thread)
    pub fn drain_commands(&self) -> Vec<IosCommand> {
        let mut commands = Vec::new();
        while let Ok(cmd) = self.receiver.try_recv() {
            commands.push(cmd);
        }
        commands
    }

    pub fn sender(&self) -> Sender<IosCommand> {
        self.sender.clone()
    }
}

impl Default for IosCommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Global command queue instance
static COMMAND_QUEUE: once_cell::sync::Lazy<IosCommandQueue> =
    once_cell::sync::Lazy::new(IosCommandQueue::new);

/// Get the global command queue sender
pub fn get_command_sender() -> Sender<IosCommand> {
    COMMAND_QUEUE.sender()
}

/// Drain commands from the global queue (called from main thread)
pub fn drain_commands() -> Vec<IosCommand> {
    COMMAND_QUEUE.drain_commands()
}
