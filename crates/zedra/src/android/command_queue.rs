/// Thread-safe command queue for Android JNI to Main Thread communication
///
/// This module solves the threading challenge where GPUI's App (containing Rc types)
/// cannot be sent between threads, but Android's JNI callbacks come from various threads.
///
/// Architecture: JNI Thread → Command Queue → Main UI Thread → GPUI App
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, bounded};
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
    /// IME text input (handles composed text like Vietnamese 'ê' from 'ee')
    ImeText { text: String },
    /// App resumed
    Resume,
    /// App paused
    Pause,
    /// App is being destroyed
    Destroy,
    /// Request a frame render
    RequestFrame,
    /// Connect to a saved host
    ConnectToHost { host_id: String },
    /// Fling gesture (velocity from Android VelocityTracker)
    Fling { velocity_x: f32, velocity_y: f32 },
    /// Soft keyboard height changed (in physical pixels, 0 = hidden)
    KeyboardHeightChanged { height: u32 },
    /// System deeplink received (zedra:// URL from intent)
    Deeplink { url: String },
}

/// Thread-safe command queue for Android
pub struct AndroidCommandQueue {
    sender: Sender<AndroidCommand>,
    receiver: Receiver<AndroidCommand>,
}

impl AndroidCommandQueue {
    /// Create a new command queue. Bounded at 512 entries to prevent OOM under
    /// sustained touch/scroll input when the main thread is slow to drain.
    pub fn new() -> Self {
        let (sender, receiver) = bounded(512);
        Self { sender, receiver }
    }

    /// Send a command from any thread (JNI callbacks).
    /// Drops the command with a warning if the queue is full (backpressure).
    pub fn send(&self, command: AndroidCommand) -> Result<()> {
        self.sender.try_send(command).map_err(|e| match e {
            crossbeam_channel::TrySendError::Full(cmd) => {
                tracing::warn!("command queue full, dropping {:?}", cmd);
                anyhow::anyhow!("command queue full")
            }
            crossbeam_channel::TrySendError::Disconnected(cmd) => {
                anyhow::anyhow!("command queue disconnected, dropping {:?}", cmd)
            }
        })
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
