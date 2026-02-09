//! Device definitions for preview sizing

use gpui::{px, Pixels, Size};

/// Predefined device sizes for previewing components
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Device {
    // iPhone
    IPhoneSE,
    IPhone14,
    #[default]
    IPhone14Pro,
    IPhone14ProMax,
    IPhone15Pro,
    IPhone15ProMax,

    // iPad
    IPadMini,
    IPadAir,
    IPadPro11,
    IPadPro13,

    // Android - Pixel
    Pixel7,
    Pixel7Pro,
    Pixel8,
    Pixel8Pro,

    // Android - Samsung
    GalaxyS23,
    GalaxyS23Ultra,
    GalaxyZFold5,

    // Generic
    Phone,
    Tablet,

    // Custom size
    Custom {
        width: u32,
        height: u32,
        scale: u32, // multiplied by 10 to avoid float (30 = 3.0x)
    },
}

impl Device {
    /// Get the logical size of the device in points/dp
    pub fn size(&self) -> Size<Pixels> {
        match self {
            // iPhone (logical points)
            Device::IPhoneSE => Size {
                width: px(375.0),
                height: px(667.0),
            },
            Device::IPhone14 => Size {
                width: px(390.0),
                height: px(844.0),
            },
            Device::IPhone14Pro => Size {
                width: px(393.0),
                height: px(852.0),
            },
            Device::IPhone14ProMax => Size {
                width: px(430.0),
                height: px(932.0),
            },
            Device::IPhone15Pro => Size {
                width: px(393.0),
                height: px(852.0),
            },
            Device::IPhone15ProMax => Size {
                width: px(430.0),
                height: px(932.0),
            },

            // iPad
            Device::IPadMini => Size {
                width: px(744.0),
                height: px(1133.0),
            },
            Device::IPadAir => Size {
                width: px(820.0),
                height: px(1180.0),
            },
            Device::IPadPro11 => Size {
                width: px(834.0),
                height: px(1194.0),
            },
            Device::IPadPro13 => Size {
                width: px(1024.0),
                height: px(1366.0),
            },

            // Android - Pixel (dp)
            Device::Pixel7 => Size {
                width: px(412.0),
                height: px(915.0),
            },
            Device::Pixel7Pro => Size {
                width: px(412.0),
                height: px(892.0),
            },
            Device::Pixel8 => Size {
                width: px(412.0),
                height: px(915.0),
            },
            Device::Pixel8Pro => Size {
                width: px(448.0),
                height: px(998.0),
            },

            // Samsung
            Device::GalaxyS23 => Size {
                width: px(360.0),
                height: px(780.0),
            },
            Device::GalaxyS23Ultra => Size {
                width: px(384.0),
                height: px(824.0),
            },
            Device::GalaxyZFold5 => Size {
                width: px(373.0),
                height: px(841.0),
            }, // Cover screen

            // Generic
            Device::Phone => Size {
                width: px(390.0),
                height: px(844.0),
            },
            Device::Tablet => Size {
                width: px(820.0),
                height: px(1180.0),
            },

            Device::Custom { width, height, .. } => Size {
                width: px(*width as f32),
                height: px(*height as f32),
            },
        }
    }

    /// Get the scale factor of the device
    pub fn scale(&self) -> f32 {
        match self {
            Device::IPhoneSE => 2.0,
            Device::IPhone14 => 3.0,
            Device::IPhone14Pro => 3.0,
            Device::IPhone14ProMax => 3.0,
            Device::IPhone15Pro => 3.0,
            Device::IPhone15ProMax => 3.0,
            Device::IPadMini => 2.0,
            Device::IPadAir => 2.0,
            Device::IPadPro11 => 2.0,
            Device::IPadPro13 => 2.0,
            Device::Pixel7 => 2.75,
            Device::Pixel7Pro => 3.5,
            Device::Pixel8 => 2.75,
            Device::Pixel8Pro => 3.0,
            Device::GalaxyS23 => 3.0,
            Device::GalaxyS23Ultra => 3.0,
            Device::GalaxyZFold5 => 3.0,
            Device::Phone => 3.0,
            Device::Tablet => 2.0,
            Device::Custom { scale, .. } => *scale as f32 / 10.0,
        }
    }

    /// Get safe area insets for the device
    pub fn safe_area(&self) -> SafeAreaInsets {
        match self {
            // iPhone with notch/dynamic island
            Device::IPhone14 | Device::IPhone14Pro | Device::IPhone14ProMax => SafeAreaInsets {
                top: px(59.0),
                bottom: px(34.0),
                left: px(0.0),
                right: px(0.0),
            },
            Device::IPhone15Pro | Device::IPhone15ProMax => SafeAreaInsets {
                top: px(59.0),
                bottom: px(34.0),
                left: px(0.0),
                right: px(0.0),
            },

            // iPhone SE (no notch)
            Device::IPhoneSE => SafeAreaInsets {
                top: px(20.0),
                bottom: px(0.0),
                left: px(0.0),
                right: px(0.0),
            },

            // iPad
            Device::IPadMini | Device::IPadAir | Device::IPadPro11 | Device::IPadPro13 => {
                SafeAreaInsets {
                    top: px(24.0),
                    bottom: px(20.0),
                    left: px(0.0),
                    right: px(0.0),
                }
            }

            // Android with punch-hole camera
            Device::Pixel7
            | Device::Pixel7Pro
            | Device::Pixel8
            | Device::Pixel8Pro
            | Device::GalaxyS23
            | Device::GalaxyS23Ultra => SafeAreaInsets {
                top: px(32.0),
                bottom: px(16.0),
                left: px(0.0),
                right: px(0.0),
            },

            Device::GalaxyZFold5 => SafeAreaInsets {
                top: px(32.0),
                bottom: px(16.0),
                left: px(0.0),
                right: px(0.0),
            },

            // Generic
            Device::Phone => SafeAreaInsets {
                top: px(44.0),
                bottom: px(34.0),
                left: px(0.0),
                right: px(0.0),
            },
            Device::Tablet => SafeAreaInsets {
                top: px(24.0),
                bottom: px(20.0),
                left: px(0.0),
                right: px(0.0),
            },

            Device::Custom { .. } => SafeAreaInsets::zero(),
        }
    }

    /// Get the device name for display
    pub fn name(&self) -> &'static str {
        match self {
            Device::IPhoneSE => "iPhone SE",
            Device::IPhone14 => "iPhone 14",
            Device::IPhone14Pro => "iPhone 14 Pro",
            Device::IPhone14ProMax => "iPhone 14 Pro Max",
            Device::IPhone15Pro => "iPhone 15 Pro",
            Device::IPhone15ProMax => "iPhone 15 Pro Max",
            Device::IPadMini => "iPad Mini",
            Device::IPadAir => "iPad Air",
            Device::IPadPro11 => "iPad Pro 11\"",
            Device::IPadPro13 => "iPad Pro 13\"",
            Device::Pixel7 => "Pixel 7",
            Device::Pixel7Pro => "Pixel 7 Pro",
            Device::Pixel8 => "Pixel 8",
            Device::Pixel8Pro => "Pixel 8 Pro",
            Device::GalaxyS23 => "Galaxy S23",
            Device::GalaxyS23Ultra => "Galaxy S23 Ultra",
            Device::GalaxyZFold5 => "Galaxy Z Fold 5",
            Device::Phone => "Generic Phone",
            Device::Tablet => "Generic Tablet",
            Device::Custom { .. } => "Custom",
        }
    }

    /// Get all available devices
    pub fn all() -> Vec<Device> {
        vec![
            Device::IPhone14Pro,
            Device::IPhone14ProMax,
            Device::IPhone15Pro,
            Device::IPhoneSE,
            Device::Pixel8Pro,
            Device::GalaxyS23Ultra,
            Device::IPadPro11,
            Device::Phone,
            Device::Tablet,
        ]
    }

    /// Get all phone devices
    pub fn phones() -> Vec<Device> {
        vec![
            Device::IPhone14Pro,
            Device::IPhone14ProMax,
            Device::IPhone15Pro,
            Device::IPhoneSE,
            Device::Pixel8,
            Device::Pixel8Pro,
            Device::GalaxyS23,
            Device::GalaxyS23Ultra,
        ]
    }
}

/// Safe area insets for a device
#[derive(Clone, Copy, Debug, Default)]
pub struct SafeAreaInsets {
    pub top: Pixels,
    pub bottom: Pixels,
    pub left: Pixels,
    pub right: Pixels,
}

impl SafeAreaInsets {
    pub fn zero() -> Self {
        Self {
            top: px(0.0),
            bottom: px(0.0),
            left: px(0.0),
            right: px(0.0),
        }
    }
}
