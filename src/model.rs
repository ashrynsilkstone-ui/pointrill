use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

pub type Dpi = (u16, u16);
pub const CYCLES: [(i32, &str); 4] = [
    (5, "24 und 48 Rastpunkte"),
    (3, "24 und Freilauf"),
    (6, "48 und Freilauf"),
    (7, "Alle drei Modi"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Device {
    pub serial: String,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
}

impl Device {
    pub fn basilisk(&self) -> bool {
        self.vid == 0x1532 && matches!(self.pid, 0xe2 | 0xe3)
    }

    pub fn viper_receiver(&self) -> bool {
        self.vid == 0x1532 && self.pid == 0xe6
    }

    pub fn connection(&self) -> &str {
        let name = self.name.to_ascii_lowercase();
        if name.contains("bluetooth") {
            "Bluetooth"
        } else if is_receiver(&name) {
            "USB-Empfänger"
        } else {
            "USB"
        }
    }
}

pub fn is_receiver(name: &str) -> bool {
    [
        "wireless",
        "receiver",
        "dongle",
        "mouse dock",
        "charging dock",
    ]
    .iter()
    .any(|word| name.contains(word))
}

#[derive(Default, Debug)]
pub struct Status {
    pub battery: Option<u8>,
    pub charging: bool,
    pub dpi: Option<Dpi>,
    pub dpi_stage: Option<u8>,
    pub polling: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    Sensor,
    Wheel,
    Lights,
    Power,
    Receiver,
    Profiles,
    Diagnostics,
}

impl Page {
    pub const ALL: [Self; 7] = [
        Self::Sensor,
        Self::Wheel,
        Self::Lights,
        Self::Power,
        Self::Receiver,
        Self::Profiles,
        Self::Diagnostics,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Sensor => "Sensor",
            Self::Wheel => "Scrollrad",
            Self::Lights => "Beleuchtung",
            Self::Power => "Akku",
            Self::Receiver => "Dongle",
            Self::Profiles => "Profile",
            Self::Diagnostics => "Gerätetest",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub id: &'static str,
    pub name: &'static str,
    pub prefix: &'static str,
}

pub const ZONES: [Zone; 9] = [
    Zone {
        id: "chroma",
        name: "Gerät",
        prefix: "",
    },
    Zone {
        id: "logo",
        name: "Logo",
        prefix: "Logo",
    },
    Zone {
        id: "scroll",
        name: "Scrollrad",
        prefix: "Scroll",
    },
    Zone {
        id: "backlight",
        name: "Unterboden",
        prefix: "Backlight",
    },
    Zone {
        id: "left",
        name: "Linke Zone",
        prefix: "Left",
    },
    Zone {
        id: "right",
        name: "Rechte Zone",
        prefix: "Right",
    },
    Zone {
        id: "charging",
        name: "Laden",
        prefix: "Charging",
    },
    Zone {
        id: "fast_charging",
        name: "Schnellladen",
        prefix: "FastCharging",
    },
    Zone {
        id: "fully_charged",
        name: "Voll geladen",
        prefix: "FullyCharged",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    Dpi,
    Stages,
    Polling,
    Wheel,
    Cycle(u8),
    Smart,
    Sensitivity(u8),
    Acceleration,
    Sleep,
    Warning,
    Brightness(Option<Zone>),
    Light(Zone),
    Indicator,
    Indicators,
    ViperIndicators,
}

impl Key {
    pub fn assist(&self) -> bool {
        matches!(
            self,
            Self::Smart | Self::Acceleration | Self::Sensitivity(_)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Off,
    Static,
    Spectrum,
    Wave,
    Breath,
    BreathRandom,
    BreathMono,
    Blinking,
    Pulsate,
    On,
    Wheel,
}

impl Effect {
    pub const ALL: [Self; 11] = [
        Self::Off,
        Self::Static,
        Self::Spectrum,
        Self::Wave,
        Self::Breath,
        Self::BreathRandom,
        Self::BreathMono,
        Self::Blinking,
        Self::Pulsate,
        Self::On,
        Self::Wheel,
    ];

    pub fn raw(self) -> &'static str {
        match self {
            Self::Off => "none",
            Self::Static => "static",
            Self::Spectrum => "spectrum",
            Self::Wave => "wave",
            Self::Breath => "breath_single",
            Self::BreathRandom => "breath_random",
            Self::BreathMono => "breath_mono",
            Self::Blinking => "blinking",
            Self::Pulsate => "pulsate",
            Self::On => "on",
            Self::Wheel => "wheel",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Off => "Aus",
            Self::Static => "Statisch",
            Self::Spectrum => "Farbwechsel",
            Self::Wave => "Welle",
            Self::Breath => "Atmen",
            Self::BreathRandom => "Atmen · Farbwechsel",
            Self::BreathMono => "Einfarbig atmen",
            Self::Blinking => "Blinken",
            Self::Pulsate => "Pulsieren",
            Self::On => "An",
            Self::Wheel => "Farbrad",
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::Off => "None",
            Self::Static => "Static",
            Self::Spectrum => "Spectrum",
            Self::Wave => "Wave",
            Self::Breath => "BreathSingle",
            Self::BreathRandom => "BreathRandom",
            Self::BreathMono => "BreathMono",
            Self::Blinking => "Blinking",
            Self::Pulsate => "Pulsate",
            Self::On => "On",
            Self::Wheel => "Wheel",
        }
    }

    pub fn color(self) -> bool {
        matches!(
            self,
            Self::Static | Self::Breath | Self::Blinking | Self::Pulsate
        )
    }

    pub fn direction(self) -> bool {
        matches!(self, Self::Wave | Self::Wheel)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    Bool(bool),
    Number(i32),
    Dpi(Dpi),
    Stages {
        active: u8,
        points: Vec<Dpi>,
    },
    Light {
        effect: Option<Effect>,
        color: [u8; 3],
        direction: i32,
    },
    Indicators([i32; 3]),
}

impl Value {
    pub fn current_dpi(&self) -> Option<Dpi> {
        match self {
            Self::Dpi(value) => Some(*value),
            Self::Stages { active, points } => {
                points.get(usize::from(active.saturating_sub(1))).copied()
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Control {
    Switch,
    Choices(Vec<(i32, String)>),
    Range {
        min: i32,
        max: i32,
        unit: &'static str,
    },
    Dpi {
        max: u16,
        x_only: bool,
        presets: Vec<u16>,
    },
    Stages {
        max: u16,
    },
    Effects(Vec<Effect>),
    Indicators(Vec<(i32, String)>),
}

#[derive(Clone, Debug)]
pub struct Field {
    pub key: Key,
    pub label: &'static str,
    pub page: Page,
    pub control: Control,
    pub value: Value,
}

impl Field {
    pub fn validates(&self, value: &Value) -> Result<()> {
        match (&self.control, value) {
            (Control::Switch, Value::Bool(_)) => {}
            (Control::Choices(options), Value::Number(n)) => {
                ensure!(options.iter().any(|(v, _)| v == n), "Ungültige Option")
            }
            (Control::Range { min, max, .. }, Value::Number(n)) => {
                ensure!((*min..=*max).contains(n), "Wert außerhalb des Bereichs")
            }
            (
                Control::Dpi {
                    max,
                    x_only,
                    presets,
                },
                Value::Dpi((x, y)),
            ) => {
                ensure!(
                    (100..=*max).contains(x) || presets.contains(x),
                    "DPI X außerhalb des Bereichs"
                );
                ensure!(
                    presets.is_empty() || presets.contains(x),
                    "Diese DPI-Stufe fehlt im Gerät"
                );
                ensure!(
                    if *x_only {
                        *y == 0
                    } else {
                        (100..=*max).contains(y)
                    },
                    "DPI Y außerhalb des Bereichs"
                );
            }
            (Control::Stages { max }, Value::Stages { active, points }) => {
                ensure!(
                    (1..=5).contains(&points.len())
                        && (1..=points.len()).contains(&usize::from(*active)),
                    "Ungültige DPI-Stufe"
                );
                ensure!(
                    points
                        .iter()
                        .all(|(x, y)| (100..=*max).contains(x) && (100..=*max).contains(y)),
                    "DPI außerhalb des Bereichs"
                );
            }
            (
                Control::Effects(effects),
                Value::Light {
                    effect: Some(effect),
                    direction,
                    ..
                },
            ) => {
                ensure!(
                    effects.contains(effect),
                    "Effekt wird vom Gerät nicht unterstützt"
                );
                ensure!(
                    !effect.direction() || matches!(direction, 1 | 2),
                    "Ungültige Effektrichtung"
                );
            }
            (Control::Indicators(options), Value::Indicators(values)) => ensure!(
                values.iter().all(|v| options.iter().any(|(n, _)| n == v)),
                "Ungültiger LED-Modus"
            ),
            _ => bail!("Ungültiger Einstellungstyp"),
        }
        Ok(())
    }
}

pub struct Snapshot {
    pub device: Device,
    pub fields: Vec<Field>,
    pub volatile_dpi: bool,
    pub product_image: Option<String>,
}

pub fn has_free_spin(value: &Value) -> bool {
    matches!(value, Value::Number(n) if *n > 0 && *n & 2 != 0)
}

pub fn draft_changes(fields: &[Field], draft: &[Value]) -> Vec<(Key, Value)> {
    let free_spin = fields
        .iter()
        .zip(draft)
        .find(|(field, _)| matches!(field.key, Key::Cycle(_)))
        .is_none_or(|(_, value)| has_free_spin(value));
    fields
        .iter()
        .zip(draft)
        .filter(|(field, value)| field.value != **value && (free_spin || !field.key.assist()))
        .map(|(field, value)| (field.key.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_tactile_draft_never_submits_hidden_assists() {
        let fields = [
            Field {
                key: Key::Cycle(1),
                label: "Cycle",
                page: Page::Wheel,
                control: Control::Choices(vec![]),
                value: Value::Number(7),
            },
            Field {
                key: Key::Smart,
                label: "Smart",
                page: Page::Wheel,
                control: Control::Switch,
                value: Value::Bool(false),
            },
        ];
        assert_eq!(
            draft_changes(&fields, &[Value::Number(5), Value::Bool(true)]),
            vec![(Key::Cycle(1), Value::Number(5))]
        );
        assert_eq!(
            draft_changes(&fields, &[Value::Number(7), Value::Bool(true)]),
            vec![(Key::Smart, Value::Bool(true))]
        );
    }
}
