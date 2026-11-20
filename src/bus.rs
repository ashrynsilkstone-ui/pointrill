use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use zbus::blocking::{Connection, Proxy, connection};
use zbus::zvariant::{DynamicDeserialize, DynamicType};

use crate::hid::Hid;
use crate::model::*;

const MISC: &str = "razer.device.misc";
const SENSOR: &str = "razer.device.dpi";
const SCROLL: &str = "razer.device.scroll";
const POWER: &str = "razer.device.power";
const LIGHT: &str = "razer.device.lighting";
const INDICATOR: [&str; 5] = ["Aus", "An", "Akku", "Verbindung", "Kopplung"];
const MULTI_INDICATOR: [&str; 5] = ["Aus", "Akku", "Verbindung", "Pollingrate", "DPI"];

pub(crate) type Methods = HashSet<(String, String)>;

pub struct OpenRazer {
    connection: Connection,
    owner: String,
    catalog: HashMap<String, Option<Device>>,
    cycles: CycleCache,
}

impl OpenRazer {
    pub fn connect() -> Result<Self> {
        let connection = connection::Builder::session()?
            .method_timeout(Duration::from_secs(3))
            .build()?;
        Ok(Self {
            connection,
            owner: String::new(),
            catalog: HashMap::new(),
            cycles: CycleCache::load(),
        })
    }

    fn bus_owner(&self) -> Result<String> {
        let bus = Proxy::new(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )?;
        bus.call("GetNameOwner", &("org.razer",))
            .context("OpenRazer ist gerade nicht erreichbar")
    }

    fn call<R, B>(&self, serial: &str, interface: &str, method: &str, args: &B) -> Result<R>
    where
        R: for<'d> DynamicDeserialize<'d>,
        B: Serialize + DynamicType,
    {
        let path = format!("/org/razer/device/{serial}");
        let proxy = Proxy::new(
            &self.connection,
            self.owner.as_str(),
            path.as_str(),
            interface,
        )?;
        proxy.call(method, args).with_context(|| method.to_owned())
    }

    fn get<R: for<'d> DynamicDeserialize<'d>>(
        &self,
        device: &Device,
        interface: &str,
        method: &str,
    ) -> Result<R> {
        self.call(&device.serial, interface, method, &())
    }

    fn set<B: Serialize + DynamicType>(
        &self,
        device: &Device,
        interface: &str,
        method: &str,
        args: &B,
    ) -> Result<()> {
        self.call(&device.serial, interface, method, args)
    }

    pub fn scan(&mut self) -> Result<(Vec<Device>, bool)> {
        let owner = self.bus_owner()?;
        let restarted = owner != self.owner;
        if restarted {
            self.catalog.clear();
            self.owner = owner;
        }
        let root = Proxy::new(
            &self.connection,
            self.owner.as_str(),
            "/org/razer",
            "razer.devices",
        )?;
        let serials: Vec<String> = root.call("getDevices", &())?;
        self.catalog.retain(|serial, _| serials.contains(serial));
        let mut devices = Vec::new();
        for serial in serials {
            if let Some(entry) = self.catalog.get(&serial) {
                if let Some(device) = entry {
                    if self.identity(device).is_ok() {
                        devices.push(device.clone());
                    } else {
                        self.catalog.remove(&serial);
                    }
                }
                if self.catalog.contains_key(&serial) {
                    continue;
                }
            }
            if let Ok(entry) = self.describe(&serial) {
                devices.extend(entry.iter().cloned());
                self.catalog.insert(serial, entry);
            }
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name).then(a.serial.cmp(&b.serial)));
        Ok((devices, restarted))
    }

    fn describe(&self, serial: &str) -> Result<Option<Device>> {
        let kind: String = self.call(serial, MISC, "getDeviceType", &())?;
        if !matches!(kind.as_str(), "mouse" | "accessory" | "dongle") {
            return Ok(None);
        }
        let name: String = self.call(serial, MISC, "getDeviceName", &())?;
        if kind != "mouse" && !is_receiver(&name.to_ascii_lowercase()) {
            return Ok(None);
        }
        let ids: Vec<i32> = self.call(serial, MISC, "getVidPid", &())?;
        ensure!(ids.len() == 2, "OpenRazer meldet keine gültige Geräte-ID");
        Ok(Some(Device {
            serial: serial.to_owned(),
            name,
            vid: ids[0].try_into()?,
            pid: ids[1].try_into()?,
        }))
    }

    fn identity(&self, device: &Device) -> Result<()> {
        let ids: Vec<i32> = self.get(device, MISC, "getVidPid")?;
        ensure!(
            ids == [i32::from(device.vid), i32::from(device.pid)],
            "Das angeschlossene Gerät hat sich geändert"
        );
        Ok(())
    }

    fn methods(&self, device: &Device) -> Result<Methods> {
        let xml: String = self.get(device, "org.freedesktop.DBus.Introspectable", "Introspect")?;
        let doc = roxmltree::Document::parse_with_options(
            &xml,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )?;
        Ok(doc
            .descendants()
            .filter(|node| node.has_tag_name("interface"))
            .flat_map(|interface| {
                let name = interface.attribute("name").unwrap_or_default();
                interface
                    .children()
                    .filter(|node| node.has_tag_name("method"))
                    .filter_map(move |node| {
                        Some((name.to_owned(), node.attribute("name")?.to_owned()))
                    })
            })
            .collect())
    }

    pub fn dpi(&self, device: &Device) -> Result<Dpi> {
        let points: Vec<i32> = self.get(device, SENSOR, "getDPI")?;
        let x = points.first().context("Keine DPI-Rückmeldung")?;
        let y = points.get(1).copied().unwrap_or(0);
        Ok(((*x).try_into()?, y.try_into()?))
    }

    pub fn status(&self, device: &Device, methods: &Methods) -> Status {
        let battery = available(methods, POWER, "getBattery")
            .then(|| self.get::<f64>(device, POWER, "getBattery").ok())
            .flatten()
            .filter(|n| (0.0..=100.0).contains(n))
            .map(|n| n.round() as u8);
        let charging = battery.is_some()
            && available(methods, POWER, "isCharging")
            && self.get(device, POWER, "isCharging").unwrap_or(false);
        let dpi = available(methods, SENSOR, "getDPI")
            .then(|| self.dpi(device).ok())
            .flatten();
        let polling = available(methods, MISC, "getPollRate")
            .then(|| self.get::<i32>(device, MISC, "getPollRate").ok())
            .flatten()
            .filter(|n| *n > 0);
        Status {
            battery,
            charging,
            dpi,
            polling,
        }
    }

    pub fn describe_fields(&self, device: Device) -> Result<(Snapshot, Methods)> {
        self.identity(&device)?;
        let methods = self.methods(&device)?;
        let (mut builder, mut volatile_dpi) = (Fields::new(self, &device, &methods), false);
        if let Ok(max) = self
            .get::<i32>(&device, SENSOR, "maxDPI")
            .and_then(|n| Ok(u16::try_from(n)?))
        {
            builder.supported(
                Key::Stages,
                "DPI-Stufen",
                Page::Sensor,
                Control::Stages { max },
                SENSOR,
                "getDPIStages",
                "setDPIStages",
            );
            if !builder.fields.iter().any(|field| field.key == Key::Stages) {
                let presets: Vec<u16> = if available(&methods, SENSOR, "availableDPI") {
                    self.get::<Vec<i32>>(&device, SENSOR, "availableDPI")
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|n| n.try_into().ok())
                        .collect()
                } else {
                    vec![]
                };
                let x_only = !presets.is_empty() || self.dpi(&device).is_ok_and(|p| p.1 == 0);
                builder.supported(
                    Key::Dpi,
                    "Empfindlichkeit",
                    Page::Sensor,
                    Control::Dpi {
                        max,
                        x_only,
                        presets,
                    },
                    SENSOR,
                    "getDPI",
                    "setDPI",
                );
            }
        }
        if available(&methods, MISC, "getSupportedPollRates")
            && let Ok(rates) = self.get::<Vec<u16>>(&device, MISC, "getSupportedPollRates")
        {
            let mut choices: Vec<_> = rates
                .into_iter()
                .filter(|rate| *rate > 0)
                .map(|rate| (i32::from(rate), format!("{rate} Hz")))
                .collect();
            choices.sort_by_key(|(rate, _)| *rate);
            choices.dedup_by_key(|(rate, _)| *rate);
            if !choices.is_empty() {
                builder.supported(
                    Key::Polling,
                    "Pollingrate",
                    Page::Sensor,
                    Control::Choices(choices),
                    MISC,
                    "getPollRate",
                    "setPollRate",
                );
            }
        }
        if device.basilisk() {
            if let Ok(hid) = Hid::open(&device) {
                volatile_dpi = available(&methods, SENSOR, "getDPI");
                if let Ok(profile) = hid.profile() {
                    builder.push(
                        Key::Cycle(profile),
                        "Modi beim Umschalten",
                        Page::Wheel,
                        Control::Choices(choices(&CYCLES)),
                    );
                    builder.push(
                        Key::Sensitivity(profile),
                        "Smart-Reel-Empfindlichkeit",
                        Page::Wheel,
                        Control::Choices((0..5).map(|n| (n, format!("Stufe {}", n + 1))).collect()),
                    );
                }
            }
        } else {
            builder.supported(
                Key::Wheel,
                "Radmodus",
                Page::Wheel,
                Control::Choices(choices(&[(0, "Rastend"), (1, "Freilauf")])),
                SCROLL,
                "getScrollMode",
                "setScrollMode",
            );
        }
        if !device.basilisk()
            || builder
                .fields
                .iter()
                .any(|f| matches!(f.key, Key::Cycle(_)))
        {
            builder.supported(
                Key::Smart,
                "Smart Reel",
                Page::Wheel,
                Control::Switch,
                SCROLL,
                "getScrollSmartReel",
                "setScrollSmartReel",
            );
            builder.supported(
                Key::Acceleration,
                "Scrollbeschleunigung",
                Page::Wheel,
                Control::Switch,
                SCROLL,
                "getScrollAcceleration",
                "setScrollAcceleration",
            );
        }
        builder.supported(
            Key::Sleep,
            "Ruhezustand nach",
            Page::Power,
            Control::Range {
                min: 60,
                max: 900,
                unit: "s",
            },
            POWER,
            "getIdleTime",
            "setIdleTime",
        );
        builder.supported(
            Key::Warning,
            "Akkuwarnung",
            Page::Power,
            Control::Range {
                min: 5,
                max: 25,
                unit: "%",
            },
            POWER,
            "getLowBatteryThreshold",
            "setLowBatteryThreshold",
        );
        builder.supported(
            Key::Brightness(None),
            "Gesamthelligkeit",
            Page::Lights,
            Control::Range {
                min: 0,
                max: 100,
                unit: "%",
            },
            "razer.device.lighting.brightness",
            "getBrightness",
            "setBrightness",
        );
        for zone in ZONES {
            let interface = zone_interface(zone);
            let enabled: Vec<Effect> = Effect::ALL
                .into_iter()
                .filter(|effect| {
                    available(
                        &methods,
                        &interface,
                        &format!("set{}{}", zone.prefix, effect.method()),
                    )
                })
                .collect();
            if !enabled.is_empty()
                && available(&methods, &interface, &format!("get{}Effect", zone.prefix))
            {
                builder.push(
                    Key::Light(zone),
                    zone.name,
                    Page::Lights,
                    Control::Effects(enabled),
                );
            }
            let getter = format!("get{}Brightness", zone.prefix);
            let setter = format!("set{}Brightness", zone.prefix);
            builder.supported(
                Key::Brightness(Some(zone)),
                zone.name,
                Page::Lights,
                Control::Range {
                    min: 0,
                    max: 100,
                    unit: "%",
                },
                &interface,
                &getter,
                &setter,
            );
        }
        builder.supported(
            Key::Indicator,
            "Status-LED",
            Page::Receiver,
            Control::Choices(labels(&INDICATOR)),
            MISC,
            "getHyperPollingLED",
            "setHyperPollingLED",
        );
        builder.supported(
            Key::Indicators,
            "Status-LEDs",
            Page::Receiver,
            Control::Indicators(labels(&MULTI_INDICATOR)),
            MISC,
            "getHyperPollingMultiLEDModes",
            "setHyperPollingMultiLEDModes",
        );
        if device.viper_receiver() && !builder.fields.iter().any(|f| f.key == Key::Indicators) {
            builder.push(
                Key::ViperIndicators,
                "Status-LEDs",
                Page::Receiver,
                Control::Indicators(labels(&MULTI_INDICATOR)),
            );
        }
        let advertised = if available(&methods, MISC, "getDeviceImage") {
            self.get::<String>(&device, MISC, "getDeviceImage").ok()
        } else {
            None
        };
        let product_image = crate::photos::source(&device, advertised.as_deref());
        Ok((
            Snapshot {
                fields: builder.fields,
                device,
                volatile_dpi,
                product_image,
            },
            methods,
        ))
    }

    fn read(&self, device: &Device, key: &Key) -> Result<Value> {
        Ok(match key {
            Key::Dpi => Value::Dpi(self.dpi(device)?),
            Key::Stages => {
                let (active, points): (u8, Vec<Dpi>) = self.get(device, SENSOR, "getDPIStages")?;
                ensure!(
                    (1..=5).contains(&points.len())
                        && (1..=points.len()).contains(&usize::from(active)),
                    "Das Gerät meldet keine gültigen DPI-Stufen"
                );
                Value::Stages { active, points }
            }
            Key::Polling => Value::Number(self.get(device, MISC, "getPollRate")?),
            Key::Wheel => Value::Number(i32::from(self.get::<u8>(
                device,
                SCROLL,
                "getScrollMode",
            )?)),
            Key::Cycle(profile) => {
                let mask = Hid::open(device)?.cycle(*profile)?;
                Value::Number(if matches!(mask, 3 | 5 | 6 | 7) {
                    mask.into()
                } else {
                    self.cycles.get(device, *profile).map_or(-1, i32::from)
                })
            }
            Key::Sensitivity(profile) => {
                Value::Number(Hid::open(device)?.sensitivity(*profile)?.into())
            }
            Key::Smart => Value::Bool(self.get(device, SCROLL, "getScrollSmartReel")?),
            Key::Acceleration => Value::Bool(self.get(device, SCROLL, "getScrollAcceleration")?),
            Key::Sleep => Value::Number(self.get::<u16>(device, POWER, "getIdleTime")?.into()),
            Key::Warning => Value::Number(
                self.get::<u8>(device, POWER, "getLowBatteryThreshold")?
                    .into(),
            ),
            Key::Brightness(zone) => {
                let (interface, prefix) = brightness_location(*zone);
                let brightness: f64 =
                    self.get(device, &interface, &format!("get{prefix}Brightness"))?;
                Value::Number(brightness.round().clamp(0.0, 100.0) as i32)
            }
            Key::Light(zone) => {
                let interface = zone_interface(*zone);
                let prefix = zone.prefix;
                let effect_name: String =
                    self.get(device, &interface, &format!("get{prefix}Effect"))?;
                let effect = Effect::ALL
                    .into_iter()
                    .find(|effect| effect.raw() == effect_name);
                let color = if effect.is_some_and(Effect::color) {
                    self.get::<Vec<u8>>(device, &interface, &format!("get{prefix}EffectColors"))?
                        .get(..3)
                        .context("Keine RGB-Rückmeldung")?
                        .try_into()?
                } else {
                    [101, 238, 211]
                };
                let direction = if effect.is_some_and(Effect::direction) {
                    self.get(device, &interface, &format!("get{prefix}WaveDir"))?
                } else {
                    1
                };
                Value::Light {
                    effect,
                    color,
                    direction,
                }
            }
            Key::Indicator => {
                Value::Number(self.get::<u8>(device, MISC, "getHyperPollingLED")?.into())
            }
            Key::Indicators => {
                let data: Vec<u8> = self.get(device, MISC, "getHyperPollingMultiLEDModes")?;
                let modes: [u8; 3] = data
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Drei LED-Werte erwartet"))?;
                Value::Indicators(modes.map(i32::from))
            }
            Key::ViperIndicators => Value::Indicators(Hid::open(device)?.indicators()?),
        })
    }

    fn write(&mut self, device: &Device, key: &Key, value: &Value) -> Result<()> {
        match (key, value) {
            (Key::Dpi, Value::Dpi(pair)) => self.set(device, SENSOR, "setDPI", pair)?,
            (Key::Stages, Value::Stages { active, points }) => {
                self.set(device, SENSOR, "setDPIStages", &(*active, points))?;
                self.set(device, SENSOR, "setDPI", &points[usize::from(*active) - 1])?;
            }
            (Key::Polling, Value::Number(n)) => {
                self.set(device, MISC, "setPollRate", &(*n as u16,))?
            }
            (Key::Wheel, Value::Number(n)) => {
                self.set(device, SCROLL, "setScrollMode", &(*n as u8,))?
            }
            (Key::Cycle(slot), Value::Number(n)) => {
                Hid::open(device)?.change_profile(*slot, true, *n as u8)?;
                self.cycles.put(device, *slot, *n as u8);
                let mode: u8 = self.get(device, SCROLL, "getScrollMode")?;
                if mode > 2 || *n & (1 << mode) == 0 {
                    let next: u8 = [0, 2, 1]
                        .into_iter()
                        .find(|mode| *n & (1 << mode) != 0)
                        .context("Keine Radmodi ausgewählt")?;
                    self.set(device, SCROLL, "setScrollMode", &(next,))?;
                    sleep(Duration::from_millis(250));
                    let mode: u8 = self.get(device, SCROLL, "getScrollMode")?;
                    ensure!(
                        mode <= 2 && *n & (1 << mode) != 0,
                        "Die Maus hat keinen freigegebenen Radmodus übernommen"
                    );
                }
            }
            (Key::Sensitivity(slot), Value::Number(n)) => {
                Hid::open(device)?.change_profile(*slot, false, *n as u8)?
            }
            (Key::Smart, Value::Bool(on)) => {
                self.set(device, SCROLL, "setScrollSmartReel", &(*on,))?
            }
            (Key::Acceleration, Value::Bool(on)) => {
                self.set(device, SCROLL, "setScrollAcceleration", &(*on,))?
            }
            (Key::Sleep, Value::Number(n)) => {
                self.set(device, POWER, "setIdleTime", &(*n as u16,))?
            }
            (Key::Warning, Value::Number(n)) => {
                self.set(device, POWER, "setLowBatteryThreshold", &(*n as u8,))?
            }
            (Key::Brightness(zone), Value::Number(n)) => {
                let (interface, prefix) = brightness_location(*zone);
                self.set(
                    device,
                    &interface,
                    &format!("set{prefix}Brightness"),
                    &(*n as f64,),
                )?;
            }
            (
                Key::Light(zone),
                Value::Light {
                    effect: Some(effect),
                    color: [red, green, blue],
                    direction,
                },
            ) => {
                let path = zone_interface(*zone);
                let method = format!("set{}{}", zone.prefix, effect.method());
                if effect.color() {
                    self.set(device, &path, &method, &(*red, *green, *blue))?;
                } else if effect.direction() {
                    self.set(device, &path, &method, &(*direction,))?;
                } else {
                    self.set(device, &path, &method, &())?;
                }
            }
            (Key::Indicator, Value::Number(n)) => {
                self.set(device, MISC, "setHyperPollingLED", &(*n as u8,))?
            }
            (Key::Indicators, Value::Indicators([a, b, c])) => {
                self.set(
                    device,
                    MISC,
                    "setHyperPollingMultiLEDModes",
                    &(*a as u8, *b as u8, *c as u8),
                )?;
            }
            (Key::ViperIndicators, Value::Indicators(data)) => {
                Hid::open(device)?.set_indicators(*data)?
            }
            _ => bail!("Diese Einstellung kann nicht geschrieben werden"),
        }
        Ok(())
    }

    pub fn apply(&mut self, snapshot: &Snapshot, mut edits: Vec<(Key, Value)>) -> Result<()> {
        let device = &snapshot.device;
        ensure!(
            self.bus_owner()? == self.owner,
            "OpenRazer wurde neu gestartet. Lade das Gerät neu"
        );
        self.identity(device)?;
        if let Some(cycle_field) = snapshot
            .fields
            .iter()
            .find(|field| matches!(field.key, Key::Cycle(_)))
            && edits
                .iter()
                .any(|(key, _)| key == &cycle_field.key || key.assist())
        {
            let current = self.read(device, &cycle_field.key)?;
            let target = edits
                .iter()
                .find(|(key, _)| key == &cycle_field.key)
                .map(|(_, value)| value)
                .unwrap_or(&current);
            ensure!(
                has_free_spin(target)
                    || edits
                        .iter()
                        .all(|(key, value)| !key.assist() || *value == Value::Bool(false)),
                "Wähle zum Umschalten eine Kombination mit Freilauf"
            );
            if *target == Value::Number(5) && edits.iter().any(|(key, _)| key == &cycle_field.key) {
                for key in [Key::Smart, Key::Acceleration] {
                    if snapshot.fields.iter().any(|field| field.key == key)
                        && !edits.iter().any(|(field, _)| *field == key)
                    {
                        edits.push((key, Value::Bool(false)));
                    }
                }
            }
        }
        for (key, value) in &edits {
            let field = snapshot
                .fields
                .iter()
                .find(|field| &field.key == key)
                .context("Einstellung ist nicht verfügbar")?;
            field.validates(value).with_context(|| field.label)?;
        }
        edits.sort_by_key(|(key, _)| (!matches!(key, Key::Cycle(_)), key.assist()));
        let history: Vec<Value> = edits
            .iter()
            .map(|(key, _)| self.read(device, key))
            .collect::<Result<_>>()?;
        let mut attempted = Vec::new();
        for (i, (key, value)) in edits.iter().enumerate() {
            if matches_read(key, &history[i], value) {
                continue;
            }
            attempted.push(i);
            let written = self.write(device, key, value).and_then(|()| {
                if *key == Key::Polling {
                    sleep(Duration::from_millis(650));
                }
                ensure!(
                    matches_read(key, &self.read(device, key)?, value),
                    "Abweichende Rückmeldung für {key:?}"
                );
                Ok(())
            });
            if let Err(error) = written {
                let mut incomplete = false;
                for i in attempted.into_iter().rev() {
                    let key = &edits[i].0;
                    let old = &history[i];
                    let restored = self
                        .read(device, key)
                        .is_ok_and(|actual| matches_read(key, &actual, old))
                        || (!matches!(old, Value::Number(-1) | Value::Light { effect: None, .. })
                            && self.write(device, key, old).is_ok()
                            && self
                                .read(device, key)
                                .is_ok_and(|actual| matches_read(key, &actual, old)));
                    incomplete |= !restored;
                }
                if incomplete {
                    return Err(error.context(
                        "Teilweise übernommen. Einige Werte ließen sich nicht wiederherstellen",
                    ));
                }
                return Err(error);
            }
        }
        Ok(())
    }
}

struct Fields<'a> {
    backend: &'a OpenRazer,
    device: &'a Device,
    methods: &'a Methods,
    fields: Vec<Field>,
}

impl<'a> Fields<'a> {
    fn new(backend: &'a OpenRazer, device: &'a Device, methods: &'a Methods) -> Self {
        Self {
            backend,
            device,
            methods,
            fields: vec![],
        }
    }

    fn push(&mut self, key: Key, label: &'static str, page: Page, control: Control) {
        if let Ok(value) = self.backend.read(self.device, &key) {
            self.fields.push(Field {
                key,
                label,
                page,
                control,
                value,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn supported(
        &mut self,
        key: Key,
        label: &'static str,
        page: Page,
        control: Control,
        interface: &str,
        get: &str,
        set: &str,
    ) {
        if available(self.methods, interface, get) && available(self.methods, interface, set) {
            self.push(key, label, page, control);
        }
    }
}

fn available(methods: &Methods, interface: &str, method: &str) -> bool {
    methods.contains(&(interface.to_owned(), method.to_owned()))
}

fn brightness_location(zone: Option<Zone>) -> (String, &'static str) {
    match zone {
        Some(zone) => (zone_interface(zone), zone.prefix),
        None => (format!("{LIGHT}.brightness"), ""),
    }
}

fn zone_interface(zone: Zone) -> String {
    format!("{LIGHT}.{}", zone.id)
}

fn choices(pairs: &[(i32, &str)]) -> Vec<(i32, String)> {
    pairs
        .iter()
        .map(|(n, label)| (*n, (*label).to_owned()))
        .collect()
}

fn labels(labels: &[&str]) -> Vec<(i32, String)> {
    labels
        .iter()
        .enumerate()
        .map(|(n, label)| (n as i32, (*label).to_owned()))
        .collect()
}

fn matches_read(key: &Key, actual: &Value, expected: &Value) -> bool {
    match (key, actual, expected) {
        (Key::Brightness(_), Value::Number(a), Value::Number(b)) => (a - b).abs() <= 1,
        (
            Key::Light(_),
            Value::Light {
                effect: a,
                color: ac,
                direction: ad,
            },
            Value::Light {
                effect: b,
                color: bc,
                direction: bd,
            },
        ) => {
            a == b
                && (!a.is_some_and(Effect::color) || ac == bc)
                && (!a.is_some_and(Effect::direction) || ad == bd)
        }
        _ => actual == expected,
    }
}

struct CycleCache {
    directory: Option<PathBuf>,
    saved: BTreeMap<String, u8>,
}

impl CycleCache {
    fn load() -> Self {
        let directory = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        let saved = directory
            .as_ref()
            .and_then(|path| fs::read(path.join("Pointrill/cycles.json")).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { directory, saved }
    }

    fn key(device: &Device, profile: u8) -> String {
        format!("{}/{:04x}/{profile}", device.serial, device.pid)
    }

    fn get(&self, device: &Device, profile: u8) -> Option<u8> {
        let key = Self::key(device, profile);
        if let Some(value) = self.saved.get(&key).filter(|n| matches!(n, 3 | 5 | 6 | 7)) {
            return Some(*value);
        }
        let config = self.directory.as_ref()?;
        let legacy = key.replace('/', "\\");
        let tray = if device.pid == 0xe2 {
            format!("wired\\{}\\{profile}", device.serial)
        } else {
            format!("{}\\{profile}", device.serial)
        };
        read_legacy(&config.join("Pointrill/Pointrill.conf"), "cycle", &legacy).or_else(|| {
            read_legacy(
                &config.join("BasiliskV4Tray/BasiliskV4Tray.conf"),
                "scroll-cycle",
                &tray,
            )
        })
    }

    fn put(&mut self, device: &Device, profile: u8, value: u8) {
        self.saved.insert(Self::key(device, profile), value);
        let result = (|| {
            if let Some(dir) = &self.directory {
                let dir = dir.join("Pointrill");
                fs::create_dir_all(&dir)?;
                let tmp = dir.join(format!("cycles-{}.tmp", std::process::id()));
                fs::write(&tmp, serde_json::to_vec(&self.saved)?)?;
                fs::rename(tmp, dir.join("cycles.json"))?;
            }
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(error) = result {
            eprintln!(
                "Pointrill: Umschaltkombination konnte nicht zwischengespeichert werden: {error}"
            );
        }
    }
}

fn read_legacy(path: &Path, section: &str, key: &str) -> Option<u8> {
    let contents = fs::read_to_string(path).ok()?;
    let header = format!("[{section}]");
    contents
        .lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .filter_map(|line| line.split_once('='))
        .find_map(|(name, value)| {
            (name == key)
                .then(|| value.trim().parse::<u8>().ok())
                .flatten()
                .filter(|n| matches!(n, 3 | 5 | 6 | 7))
        })
}
