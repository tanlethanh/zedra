use std::sync::{Arc, Mutex};

#[cfg(target_os = "ios")]
pub mod droplet;
pub mod overlay;

/// Trailing followers behind the droplet; each chases the one ahead of it.
pub const TRAIL_LEN: usize = 5;

/// Droplet display state in logical pixels; the overlay writes, the effect reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct DropletState {
    pub active: bool,
    pub center: (f32, f32),
    pub radius: f32,
    pub trail: [(f32, f32); TRAIL_LEN],
    pub scale_factor: f32,
    /// Body tint, inverse of the theme: white on dark, ink on light.
    pub base_color: (f32, f32, f32),
}

pub type SharedDropletState = Arc<Mutex<DropletState>>;

pub fn shared_droplet_state() -> SharedDropletState {
    Arc::new(Mutex::new(DropletState::default()))
}

/// Install or remove the droplet render effect on the window. No-op off iOS.
fn apply_droplet_effect(window: &mut gpui::Window, enabled: bool, state: &SharedDropletState) {
    #[cfg(target_os = "ios")]
    {
        if enabled {
            let effect = droplet::DropletEffect::new(state.clone());
            window.set_render_effect(Some(Box::new(gpui_ios::IosRenderEffect(Box::new(effect)))));
        } else {
            window.set_render_effect(None);
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = (window, enabled, state);
}
