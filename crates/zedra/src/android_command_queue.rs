/// Thread-safe command queue for Android JNI to Main Thread communication
///
/// This module solves the threading challenge where GPUI's App (containing Rc types)
/// cannot be sent between threads, but Android's JNI callbacks come from various threads.
///
/// Architecture: JNI Thread → Command Queue → Main UI Thread → GPUI App
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};
use jni::{JavaVM, objects::GlobalRef};
use std::sync::{Arc, Mutex};

/// Commands that can be sent from JNI threads to the main thread
#[derive(Debug)]
pub enum AndroidCommand {
    /// Initialize the GPUI platform and app
    Initialize {
        jvm: Arc<JavaVM>,
        activity: Arc<Mutex<GlobalRef>>,
    },
    /// Surface was created
    SurfaceCreated { width: u32, height: u32 },
    /// Surface dimensions changed
    SurfaceChanged { width: u32, height: u32 },
    /// Surface was destroyed
    SurfaceDestroyed,
    /// Touch event
    Touch {
        action: i32,
        x: f32,
        y: f32,
        pointer_id: i32,
    },
    /// Key event
    Key {
        action: i32,
        key_code: i32,
        unicode: i32,
    },
    /// App resumed
    Resume,
    /// App paused
    Pause,
    /// App is being destroyed
    Destroy,
    /// Request a frame render
    RequestFrame,
    /// QR code scanned - initiate pairing
    PairViaQr { qr_data: String },
    /// Connect to a saved host
    ConnectToHost { host_id: String },
}

/// Thread-safe command queue for Android
pub struct AndroidCommandQueue {
    sender: Sender<AndroidCommand>,
    receiver: Receiver<AndroidCommand>,
}

impl AndroidCommandQueue {
    /// Create a new command queue
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver }
    }

    /// Send a command from any thread (JNI callbacks)
    pub fn send(&self, command: AndroidCommand) -> Result<()> {
        self.sender
            .send(command)
            .map_err(|e| anyhow::anyhow!("Failed to send command: {}", e))
    }

    /// Drain all pending commands on the main thread
    /// Returns a vector of commands to be executed
    pub fn drain_commands(&self) -> Vec<AndroidCommand> {
        let mut commands = Vec::new();
        while let Ok(cmd) = self.receiver.try_recv() {
            commands.push(cmd);
        }
        commands
    }

    /// Get a clone of the sender for sharing across threads
    pub fn sender(&self) -> Sender<AndroidCommand> {
        self.sender.clone()
    }
}

impl Default for AndroidCommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Global command queue instance
static COMMAND_QUEUE: once_cell::sync::Lazy<AndroidCommandQueue> =
    once_cell::sync::Lazy::new(AndroidCommandQueue::new);

/// Get the global command queue sender
pub fn get_command_sender() -> Sender<AndroidCommand> {
    COMMAND_QUEUE.sender()
}

/// Drain commands from the global queue (called from main thread)
pub fn drain_commands() -> Vec<AndroidCommand> {
    COMMAND_QUEUE.drain_commands()
}
