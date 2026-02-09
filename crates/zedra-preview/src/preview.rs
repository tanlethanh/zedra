//! Preview definition for components

use crate::device::Device;
use gpui::{AnyElement, Hsla, IntoElement};
use std::sync::Arc;

/// A preview definition for a component
pub struct Preview {
    /// Name of the component
    pub name: String,
    /// Category for grouping (e.g., "Inputs", "Layout", "Screens")
    pub category: String,
    /// Description of the component
    pub description: String,
    /// Variants of the component to preview
    pub variants: Vec<PreviewVariant>,
    /// Controllable props
    pub controls: Vec<PropControl>,
    /// Default device for preview
    pub device: Device,
    /// Whether to show in dark mode by default
    pub dark_mode: bool,
    /// Whether to show device frame
    pub show_frame: bool,
    /// Background color override
    pub background: Option<Hsla>,
}

impl Preview {
    /// Create a new preview with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            category: "Components".into(),
            description: String::new(),
            variants: Vec::new(),
            controls: Vec::new(),
            device: Device::default(),
            dark_mode: false,
            show_frame: true,
            background: None,
        }
    }

    /// Set the category for this preview
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Set the description for this preview
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Add a variant with a render function
    pub fn variant<F, E>(mut self, name: impl Into<String>, render: F) -> Self
    where
        F: Fn() -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.variants.push(PreviewVariant {
            name: name.into(),
            render: Arc::new(move || render().into_any_element()),
        });
        self
    }

    /// Add a controllable prop
    pub fn control(mut self, control: PropControl) -> Self {
        self.controls.push(control);
        self
    }

    /// Add a boolean toggle control
    pub fn toggle(mut self, name: impl Into<String>, default: bool) -> Self {
        self.controls.push(PropControl {
            name: name.into(),
            value: PropValue::Bool(default),
        });
        self
    }

    /// Add a text input control
    pub fn text_control(mut self, name: impl Into<String>, default: impl Into<String>) -> Self {
        self.controls.push(PropControl {
            name: name.into(),
            value: PropValue::String(default.into()),
        });
        self
    }

    /// Add a number control
    pub fn number_control(mut self, name: impl Into<String>, default: f64) -> Self {
        self.controls.push(PropControl {
            name: name.into(),
            value: PropValue::Number(default),
        });
        self
    }

    /// Add an enum/select control
    pub fn select(
        mut self,
        name: impl Into<String>,
        options: Vec<String>,
        default: impl Into<String>,
    ) -> Self {
        self.controls.push(PropControl {
            name: name.into(),
            value: PropValue::Enum {
                selected: default.into(),
                options,
            },
        });
        self
    }

    /// Set the default device
    pub fn device(mut self, device: Device) -> Self {
        self.device = device;
        self
    }

    /// Set dark mode default
    pub fn dark_mode(mut self, enabled: bool) -> Self {
        self.dark_mode = enabled;
        self
    }

    /// Show or hide device frame
    pub fn show_frame(mut self, show: bool) -> Self {
        self.show_frame = show;
        self
    }

    /// Set background color
    pub fn background(mut self, color: impl Into<Hsla>) -> Self {
        self.background = Some(color.into());
        self
    }

    /// Create a simple preview with just a render function
    pub fn simple<F, E>(name: impl Into<String>, render: F) -> Self
    where
        F: Fn() -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        Self::new(name).variant("Default", render)
    }
}

/// A variant of a component preview
pub struct PreviewVariant {
    /// Name of the variant
    pub name: String,
    /// Render function for this variant
    pub render: Arc<dyn Fn() -> AnyElement + Send + Sync>,
}

/// A controllable prop in the preview
#[derive(Clone, Debug)]
pub struct PropControl {
    /// Name of the prop
    pub name: String,
    /// Current value
    pub value: PropValue,
}

/// Value types for prop controls
#[derive(Clone, Debug)]
pub enum PropValue {
    Bool(bool),
    String(String),
    Number(f64),
    Color(Hsla),
    Enum {
        selected: String,
        options: Vec<String>,
    },
}

impl PropValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PropValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            PropValue::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            PropValue::Number(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_color(&self) -> Option<Hsla> {
        match self {
            PropValue::Color(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<&str> {
        match self {
            PropValue::Enum { selected, .. } => Some(selected),
            _ => None,
        }
    }
}
