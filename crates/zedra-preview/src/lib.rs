//! zedra-preview - Component Preview System for GPUI Mobile
//!
//! Similar to SwiftUI Preview, Jetpack Compose Preview, or Storybook,
//! this crate provides a way to preview and test UI components in isolation.
//!
//! # Quick Start
//!
//! ```ignore
//! use zedra_preview::{Preview, PreviewApp, Device};
//!
//! // Define a preview
//! fn button_preview() -> Preview {
//!     Preview::new("Button")
//!         .category("Inputs")
//!         .variant("Primary", || {
//!             button("Submit").variant(ButtonVariant::Primary)
//!         })
//!         .variant("Secondary", || {
//!             button("Cancel").variant(ButtonVariant::Secondary)
//!         })
//! }
//!
//! // Run the preview app
//! fn main() {
//!     PreviewApp::new()
//!         .register(button_preview())
//!         .run();
//! }
//! ```

mod device;
mod preview;
mod preview_app;
mod sidebar;
mod toolbar;

pub use device::{Device, SafeAreaInsets};
pub use preview::{Preview, PreviewVariant, PropControl, PropValue};
pub use preview_app::PreviewApp;
pub use sidebar::Sidebar;
pub use toolbar::Toolbar;

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        Device, Preview, PreviewApp, PreviewVariant, PropControl, PropValue, SafeAreaInsets,
    };
}
