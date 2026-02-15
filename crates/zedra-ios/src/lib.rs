// Zedra iOS application — GPUI + Metal rendering with transport backend
//
// Architecture:
//   Obj-C AppDelegate → GPUI FFI → Application → Metal Rendering
//   Transport layer:  zedra-rpc, zedra-relay, zedra-transport, zedra-session

pub mod gpui_app;
pub mod ios_ffi;
pub mod ios_command_queue;
pub mod ios_app;
pub mod pairing;
