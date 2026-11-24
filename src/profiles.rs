use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{Device, Field, Key, Snapshot, Value, draft_changes, has_free_spin};

use crate::storage::{config_dir as config, read_json, write_json};

const SCHEMA: u8 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Profile {
    version: u8,
    pub name: String,
    pub model: String,
    pub program: Option<String>,
    values: BTreeMap<String, Value>,
}

#[derive(Debug)]
pub struct Plan {
    pub draft: Vec<Value>,
    pub changes: Vec<(Key, Value)>,
    pub skipped: Vec<String>,
}

impl Profile {
    pub fn capture(name: &str, program: Option<&str>, snapshot: &Snapshot) -> Result<Self> {
        let profile = Self {
            version: SCHEMA,
            name: name.trim().to_owned(),
            model: model(&snapshot.device),
            program: program
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned),
            values: snapshot
                .fields
                .iter()
                .filter(|field| field.validates(&field.value).is_ok())
                .map(|field| (setting(&field.key), field.value.clone()))
                .collect(),
        };
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.version == SCHEMA, "Unbekannte Profilversion");
        ensure!(
            !self.name.is_empty()
                && self.name.trim() == self.name
                && self.name.len() <= 96
                && !self.name.chars().any(char::is_control),
            "Profilname muss 1 bis 96 Bytes lang sein"
        );
        ensure!(
            !self.model.is_empty()
                && self.model.len() <= 160
                && !self.model.chars().any(char::is_control),
            "Ungültiges Mausmodell im Profil"
        );
        ensure!(
            !self.values.is_empty() && self.values.len() <= 96,
            "Das Profil enthält keine gültige Einstellungsliste"
        );
        if let Some(program) = &self.program {
            ensure!(
                !program.is_empty()
                    && program.len() <= 128
                    && program.trim() == program
                    && !program.contains('/')
                    && !program.chars().any(char::is_control),
                "Gib den Dateinamen des Spiels ein, zum Beispiel cs2"
            );
        }
        Ok(())
    }

    pub fn plan(&self, snapshot: &Snapshot) -> Result<Plan> {
        ensure!(
            self.model == model(&snapshot.device),
            "Das Profil gehört zu einer anderen Maus"
        );
        let mut draft = Vec::with_capacity(snapshot.fields.len());
        let mut skipped: BTreeSet<_> = self.values.keys().cloned().collect();
        for field in &snapshot.fields {
            let id = setting(&field.key);
            let value = self
                .values
                .get(&id)
                .filter(|value| field.validates(value).is_ok())
                .map_or(&field.value, |value| {
                    skipped.remove(&id);
                    value
                });
            draft.push(value.clone());
        }
        if snapshot
            .fields
            .iter()
            .zip(&draft)
            .any(|(field, value)| matches!(field.key, Key::Cycle(_)) && !has_free_spin(value))
        {
            for (field, value) in snapshot.fields.iter().zip(&mut draft) {
                if field.key.assist() && self.values.contains_key(&setting(&field.key)) {
                    if *value != Value::Bool(false) {
                        skipped.insert(setting(&field.key));
                    }
                    *value = field.value.clone();
                }
            }
        }
        let mut changes = draft_changes(&snapshot.fields, &draft);
        if changes
            .iter()
            .any(|(key, value)| matches!(key, Key::Cycle(_)) && *value == Value::Number(5))
        {
            for (field, value) in snapshot.fields.iter().zip(&mut draft) {
                if matches!(field.key, Key::Smart | Key::Acceleration)
                    && field.value == Value::Bool(true)
                {
                    *value = Value::Bool(false);
                    changes.push((field.key.clone(), Value::Bool(false)));
                }
            }
        }
        Ok(Plan {
            draft,
            changes,
            skipped: skipped.into_iter().collect(),
        })
    }

    pub fn id(&self) -> String {
        digest(&format!("{}\n{}", self.model, self.name))
    }

    pub fn automatic(&self, name: &str, program: Option<&str>) -> Result<Self> {
        let mut profile = self.clone();
        profile.name = name.trim().to_owned();
        profile.program = program
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned);
        profile.validate()?;
        Ok(profile)
    }
}

pub fn model(device: &Device) -> String {
    let name = device.name.trim_end();
    let base = [
        " (Wired)",
        " (Wireless)",
        " (Bluetooth)",
        " (USB)",
        " (2.4 GHz)",
    ]
    .into_iter()
    .find_map(|suffix| name.strip_suffix(suffix))
    .unwrap_or(name);
    format!("{:04x}/{}", device.vid, base.to_lowercase())
}

fn setting(key: &Key) -> String {
    match key {
        Key::Dpi => "dpi",
        Key::Stages => "dpi_stages",
        Key::Polling => "polling",
        Key::Wheel => "wheel",
        Key::Cycle(_) => "wheel_cycle",
        Key::Smart => "smart_reel",
        Key::Sensitivity(_) => "smart_reel_sensitivity",
        Key::Acceleration => "scroll_acceleration",
        Key::Sleep => "sleep",
        Key::Warning => "low_battery",
        Key::Brightness(None) => "brightness",
        Key::Brightness(Some(zone)) => return format!("brightness.{}", zone.id),
        Key::Light(zone) => return format!("lighting.{}", zone.id),
        Key::Indicator => "receiver_led",
        Key::Indicators | Key::ViperIndicators => "receiver_leds",
    }
    .to_owned()
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn device_file(device: &Device, directory: &str) -> Result<PathBuf> {
    let name = digest(&format!(
        "{}/{:04x}/{:04x}",
        device.serial, device.vid, device.pid
    ));
    Ok(config()?.join(directory).join(format!("{name}.json")))
}

pub fn import(path: &Path) -> Result<Profile> {
    let profile: Profile = read_json(path)?;
    profile.validate()?;
    Ok(profile)
}

pub fn export(profile: &Profile, path: &Path) -> Result<()> {
    let mut portable = profile.clone();
    portable.program = None;
    write_json(path, &portable, false)
}

pub fn list() -> Result<Vec<Profile>> {
    let directory = config()?.join("profiles");
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut profiles = Vec::new();
    for file in entries {
        let file = file?;
        if file
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            profiles.push(import(&file.path())?);
        }
    }
    profiles.sort_by(|a, b| (&a.name, &a.model).cmp(&(&b.name, &b.model)));
    Ok(profiles)
}

pub fn save(profile: &Profile, replace: bool) -> Result<()> {
    let directory = config()?.join("profiles");
    fs::create_dir_all(&directory)?;
    write_json(
        &directory.join(format!("{}.json", profile.id())),
        profile,
        replace,
    )
}

pub fn remove(profile: &Profile) -> Result<()> {
    Ok(fs::remove_file(
        config()?
            .join("profiles")
            .join(format!("{}.json", profile.id())),
    )?)
}

pub fn last(device: &Device) -> Result<Profile> {
    import(&device_file(device, "previous")?)
        .context("Für diese Verbindung ist noch kein vorheriger Stand gespeichert")
}

pub fn backup(snapshot: &Snapshot) -> Result<()> {
    let profile = Profile::capture("Vorheriger Stand", None, snapshot)?;
    write_json(&device_file(&snapshot.device, "previous")?, &profile, true)
}

pub fn revision(device: &Device) -> Option<SystemTime> {
    fs::metadata(device_file(device, "changed").ok()?)
        .ok()?
        .modified()
        .ok()
}

pub fn mark_changed(device: &Device) {
    if let Ok(path) = device_file(device, "changed") {
        let _ = write_json(&path, &(), true);
    }
}

pub fn lock(name: &str) -> Result<File> {
    let directory = config()?;
    fs::create_dir_all(&directory)?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(directory.join(format!("{name}.lock")))?;
    ensure!(file.metadata()?.is_file(), "Ungültige Sperrdatei");
    // SAFETY: The open descriptor owns the advisory lock until it is dropped.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("Pointrill führt gerade eine andere Änderung aus");
    }
    Ok(file)
}

#[derive(Deserialize, Serialize)]
pub struct AutomaticRun {
    pub profile_id: String,
    pub program: String,
    pub prior: Profile,
    pub applied: Profile,
    pub phase: AutoPhase,
}

#[derive(Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum AutoPhase {
    Prepared,
    Applied,
    Failed,
}

impl AutomaticRun {
    pub fn pending() -> bool {
        config()
            .ok()
            .and_then(|path| fs::read_dir(path.join("automatic")).ok())
            .is_some_and(|mut entries| {
                entries.any(|entry| {
                    entry.is_ok_and(|entry| {
                        entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "json")
                    })
                })
            })
    }

    pub fn load(device: &Device) -> Result<Option<Self>> {
        let path = device_file(device, "automatic")?;
        match path.try_exists()? {
            true => Ok(Some(read_json(&path)?)),
            false => Ok(None),
        }
    }

    pub fn save(&self, device: &Device) -> Result<()> {
        write_json(&device_file(device, "automatic")?, self, true)
    }

    pub fn remove(device: &Device) -> Result<()> {
        Ok(fs::remove_file(device_file(device, "automatic")?)?)
    }

    pub fn record_change(&mut self, plan: &Plan) -> Result<()> {
        let changed: BTreeSet<_> = plan.changes.iter().map(|(key, _)| setting(key)).collect();
        ensure!(
            changed
                .iter()
                .all(|key| self.prior.values.contains_key(key)),
            "Für dieses Spielprofil lassen sich die Ausgangswerte nicht vollständig sichern"
        );
        self.prior.values.retain(|key, _| changed.contains(key));
        self.applied.values = plan
            .changes
            .iter()
            .map(|(key, value)| (setting(key), value.clone()))
            .collect();
        Ok(())
    }

    pub fn restore(&self, snapshot: &Snapshot) -> Profile {
        let mut prior = self.prior.clone();
        prior.values.retain(|key, _| {
            snapshot
                .fields
                .iter()
                .find(|field| setting(&field.key) == *key)
                .zip(self.applied.values.get(key))
                .is_some_and(|(field, applied)| {
                    crate::bus::matches_read(&field.key, &field.value, applied)
                })
        });
        prior
    }
}

pub fn running_programs() -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    // Restrict inspection to processes owned by the current desktop user.
    let uid = unsafe { libc::geteuid() };
    for entry in fs::read_dir("/proc")?.flatten() {
        if !entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
            || !entry.metadata().is_ok_and(|meta| meta.uid() == uid)
        {
            continue;
        }
        if let Ok(name) = fs::read_to_string(entry.path().join("comm")) {
            names.insert(name.trim_end().to_owned());
        }
        if let Ok(path) = fs::read_link(entry.path().join("exe"))
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            names.insert(name.to_owned());
        }
        let mut bytes = Vec::new();
        if File::open(entry.path().join("cmdline"))
            .and_then(|file| file.take(128 * 1024).read_to_end(&mut bytes))
            .is_ok()
        {
            for arg in bytes
                .split_inclusive(|byte| *byte == 0)
                .take(6)
                .filter_map(|arg| std::str::from_utf8(arg.strip_suffix(&[0])?).ok())
            {
                if arg
                    .as_bytes()
                    .last_chunk::<4>()
                    .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".exe"))
                    && let Some(name) = arg.rsplit(['/', '\\']).next()
                {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    Ok(names)
}

pub fn display(field: &Field, value: &Value) -> String {
    use crate::model::Control;
    match value {
        Value::Bool(value) => if *value { "An" } else { "Aus" }.into(),
        Value::Number(value) => match &field.control {
            Control::Choices(choices) => choices
                .iter()
                .find(|(id, _)| id == value)
                .map_or_else(|| value.to_string(), |(_, label)| label.clone()),
            Control::Range { unit, .. } => format!("{value} {unit}"),
            _ => value.to_string(),
        },
        Value::Dpi((x, y)) => {
            if *y == 0 || x == y {
                format!("{x} DPI")
            } else {
                format!("{x} × {y} DPI")
            }
        }
        Value::Stages { active, points } => {
            let list: Vec<_> = points
                .iter()
                .map(|(x, y)| {
                    if x == y {
                        x.to_string()
                    } else {
                        format!("{x} × {y}")
                    }
                })
                .collect();
            format!("Stufe {active} · {}", list.join(" / "))
        }
        Value::Light {
            effect,
            color,
            direction,
        } => match effect {
            Some(effect) if effect.color() => format!(
                "{} #{:02X}{:02X}{:02X}",
                effect.title(),
                color[0],
                color[1],
                color[2]
            ),
            Some(effect) if effect.direction() => format!(
                "{} · {}",
                effect.title(),
                if *direction == 2 { "Links" } else { "Rechts" }
            ),
            Some(effect) => effect.title().into(),
            None => "Unbekannt".into(),
        },
        Value::Indicators(values) => values
            .iter()
            .map(|value| match &field.control {
                Control::Indicators(choices) => choices
                    .iter()
                    .find(|(id, _)| id == value)
                    .map_or_else(|| value.to_string(), |(_, label)| label.clone()),
                _ => value.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" / "),
    }
}
