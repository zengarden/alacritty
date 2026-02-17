use std::fmt::{self, Formatter};

use log::{error, warn};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

#[cfg(target_os = "macos")]
use winit::platform::macos::OptionAsAlt as WinitOptionAsAlt;
use winit::window::{Fullscreen, Theme as WinitTheme, WindowLevel as WinitWindowLevel};

use alacritty_config_derive::{ConfigDeserialize, SerdeReplace};

use crate::config::LOG_TARGET_CONFIG;
use crate::config::ui_config::{Delta, Percentage};

/// Default Alacritty name, used for window title and class.
pub const DEFAULT_NAME: &str = "Alacritty";

#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct WindowConfig {
    /// Initial position.
    pub position: Option<Delta<i32>>,

    /// Draw the window with title bar / borders.
    pub decorations: Decorations,

    /// Startup mode.
    pub startup_mode: StartupMode,

    /// XEmbed parent.
    #[config(skip)]
    #[serde(skip_serializing)]
    pub embed: Option<u32>,

    /// Spread out additional padding evenly.
    pub dynamic_padding: bool,

    /// Use dynamic title.
    pub dynamic_title: bool,

    /// Information to identify a particular window.
    #[config(flatten)]
    pub identity: Identity,

    /// Background opacity from 0.0 to 1.0.
    pub opacity: Percentage,

    /// Request blur behind the window.
    pub blur: bool,

    /// Controls which `Option` key should be treated as `Alt`.
    option_as_alt: OptionAsAlt,

    /// Resize increments.
    pub resize_increments: bool,

    /// Pixel padding.
    padding: Delta<u16>,

    /// Initial dimensions.
    dimensions: Dimensions,

    /// System decorations theme variant.
    decorations_theme_variant: Option<Theme>,

    /// Window level.
    pub level: WindowLevel,

    /// Tab-related options.
    pub tabs: TabsConfig,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            dynamic_title: true,
            blur: Default::default(),
            embed: Default::default(),
            padding: Default::default(),
            opacity: Default::default(),
            position: Default::default(),
            identity: Default::default(),
            dimensions: Default::default(),
            decorations: Default::default(),
            startup_mode: Default::default(),
            dynamic_padding: Default::default(),
            resize_increments: Default::default(),
            decorations_theme_variant: Default::default(),
            option_as_alt: Default::default(),
            level: Default::default(),
            tabs: Default::default(),
        }
    }
}

impl WindowConfig {
    #[inline]
    pub fn dimensions(&self) -> Option<Dimensions> {
        let (lines, columns) = (self.dimensions.lines, self.dimensions.columns);
        let (lines_is_non_zero, columns_is_non_zero) = (lines != 0, columns != 0);

        if lines_is_non_zero && columns_is_non_zero {
            // Return dimensions if both `lines` and `columns` are non-zero.
            Some(self.dimensions)
        } else if lines_is_non_zero || columns_is_non_zero {
            // Warn if either `columns` or `lines` is non-zero.

            let (zero_key, non_zero_key, non_zero_value) = if lines_is_non_zero {
                ("columns", "lines", lines)
            } else {
                ("lines", "columns", columns)
            };

            warn!(
                target: LOG_TARGET_CONFIG,
                "Both `lines` and `columns` must be non-zero for `window.dimensions` to take \
                 effect. Configured value of `{zero_key}` is 0 while that of `{non_zero_key}` is {non_zero_value}",
            );

            None
        } else {
            None
        }
    }

    #[inline]
    pub fn padding(&self, scale_factor: f32) -> (f32, f32) {
        let padding_x = (f32::from(self.padding.x) * scale_factor).floor();
        let padding_y = (f32::from(self.padding.y) * scale_factor).floor();
        (padding_x, padding_y)
    }

    #[inline]
    pub fn fullscreen(&self) -> Option<Fullscreen> {
        if self.startup_mode == StartupMode::Fullscreen {
            Some(Fullscreen::Borderless(None))
        } else {
            None
        }
    }

    #[inline]
    pub fn maximized(&self) -> bool {
        self.startup_mode == StartupMode::Maximized
    }

    #[cfg(target_os = "macos")]
    pub fn option_as_alt(&self) -> WinitOptionAsAlt {
        match self.option_as_alt {
            OptionAsAlt::OnlyLeft => WinitOptionAsAlt::OnlyLeft,
            OptionAsAlt::OnlyRight => WinitOptionAsAlt::OnlyRight,
            OptionAsAlt::Both => WinitOptionAsAlt::Both,
            OptionAsAlt::None => WinitOptionAsAlt::None,
        }
    }

    pub fn theme(&self) -> Option<WinitTheme> {
        self.decorations_theme_variant.map(WinitTheme::from)
    }
}

#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Window title.
    pub title: String,

    /// Window class.
    pub class: Class,
}

impl Default for Identity {
    fn default() -> Self {
        Self { title: DEFAULT_NAME.into(), class: Default::default() }
    }
}

#[derive(ConfigDeserialize, Serialize, Default, Debug, Copy, Clone, PartialEq, Eq)]
pub enum StartupMode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
    SimpleFullscreen,
}

#[derive(ConfigDeserialize, Serialize, Default, Debug, Copy, Clone, PartialEq, Eq)]
pub enum Decorations {
    #[default]
    Full,
    Transparent,
    Buttonless,
    None,
}

#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TabsConfig {
    /// Internal/native tabs mode.
    pub mode: TabsMode,

    /// Compact mode fine-tuning.
    pub compact: CompactTabsConfig,
}

impl Default for TabsConfig {
    fn default() -> Self {
        Self { mode: Default::default(), compact: Default::default() }
    }
}

#[derive(ConfigDeserialize, Serialize, Debug, Clone, Copy, PartialEq)]
pub struct CompactTabsConfig {
    /// Override compact left inset in points.
    ///
    /// `None` uses runtime macOS traffic-light geometry.
    pub left_inset: Option<f32>,

    /// Additional vertical title offset in points.
    pub label_offset_y: f32,
}

impl Default for CompactTabsConfig {
    fn default() -> Self {
        Self { left_inset: None, label_offset_y: 0.0 }
    }
}

#[derive(ConfigDeserialize, Serialize, Default, Debug, Copy, Clone, PartialEq, Eq)]
pub enum TabsMode {
    #[default]
    Native,
    Compact,
}

/// Window Dimensions.
///
/// Newtype to avoid passing values incorrectly.
#[derive(ConfigDeserialize, Serialize, Default, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Dimensions {
    /// Window width in character columns.
    pub columns: usize,

    /// Window Height in character lines.
    pub lines: usize,
}

/// Window class hint.
#[derive(SerdeReplace, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Class {
    pub general: String,
    pub instance: String,
}

impl Class {
    pub fn new(general: impl ToString, instance: impl ToString) -> Self {
        Self { general: general.to_string(), instance: instance.to_string() }
    }
}

impl Default for Class {
    fn default() -> Self {
        Self::new(DEFAULT_NAME, DEFAULT_NAME)
    }
}

impl<'de> Deserialize<'de> for Class {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ClassVisitor;
        impl<'a> Visitor<'a> for ClassVisitor {
            type Value = Class;

            fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Self::Value { instance: value.into(), ..Self::Value::default() })
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'a>,
            {
                let mut class = Self::Value::default();

                while let Some((key, value)) = map.next_entry::<String, toml::Value>()? {
                    match key.as_str() {
                        "instance" => match String::deserialize(value) {
                            Ok(instance) => class.instance = instance,
                            Err(err) => {
                                error!(
                                    target: LOG_TARGET_CONFIG,
                                    "Config error: class.instance: {err}"
                                );
                            },
                        },
                        "general" => match String::deserialize(value) {
                            Ok(general) => class.general = general,
                            Err(err) => {
                                error!(
                                    target: LOG_TARGET_CONFIG,
                                    "Config error: class.instance: {err}"
                                );
                            },
                        },
                        key => warn!(target: LOG_TARGET_CONFIG, "Unrecognized class field: {key}"),
                    }
                }

                Ok(class)
            }
        }

        deserializer.deserialize_any(ClassVisitor)
    }
}

#[derive(ConfigDeserialize, Serialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionAsAlt {
    /// The left `Option` key is treated as `Alt`.
    OnlyLeft,

    /// The right `Option` key is treated as `Alt`.
    OnlyRight,

    /// Both `Option` keys are treated as `Alt`.
    Both,

    /// No special handling is applied for `Option` key.
    #[default]
    None,
}

/// System decorations theme variant.
#[derive(ConfigDeserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl From<Theme> for WinitTheme {
    fn from(theme: Theme) -> Self {
        match theme {
            Theme::Light => WinitTheme::Light,
            Theme::Dark => WinitTheme::Dark,
        }
    }
}

#[derive(ConfigDeserialize, Serialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowLevel {
    #[default]
    Normal,
    AlwaysOnTop,
}

impl From<WindowLevel> for WinitWindowLevel {
    fn from(level: WindowLevel) -> Self {
        match level {
            WindowLevel::Normal => WinitWindowLevel::Normal,
            WindowLevel::AlwaysOnTop => WinitWindowLevel::AlwaysOnTop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TabsMode, WindowConfig};

    #[test]
    fn tabs_mode_defaults_to_native() {
        let config: WindowConfig = toml::from_str("").expect("parse window config");
        assert_eq!(config.tabs.mode, TabsMode::Native);
    }

    #[test]
    fn tabs_mode_compact_is_deserializable() {
        let config: WindowConfig =
            toml::from_str("[tabs]\nmode = \"Compact\"\n").expect("parse compact tabs mode");
        assert_eq!(config.tabs.mode, TabsMode::Compact);
    }

    #[test]
    fn tabs_compact_defaults_are_stable() {
        let config: WindowConfig = toml::from_str("").expect("parse window config");
        assert_eq!(config.tabs.compact.left_inset, None);
        assert_eq!(config.tabs.compact.label_offset_y, 0.0);
    }

    #[test]
    fn tabs_compact_tuning_is_deserializable() {
        let config: WindowConfig = toml::from_str(
            "[tabs]\nmode = \"Compact\"\n[tabs.compact]\nleft_inset = 72.5\nlabel_offset_y = -1.5\n",
        )
        .expect("parse compact tabs tuning");
        assert_eq!(config.tabs.mode, TabsMode::Compact);
        assert_eq!(config.tabs.compact.left_inset, Some(72.5));
        assert_eq!(config.tabs.compact.label_offset_y, -1.5);
    }
}
