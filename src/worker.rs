use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use crate::{
    bus::{Methods, OpenRazer},
    hid::Hid,
    model::{Control, Device, Dpi, Effect, Key, Snapshot, Status, Value, Zone},
    profiles::{self, AutoPhase, AutomaticRun, Profile},
};

pub enum Request {
    Scan,
    Open(Device),
    Apply(Vec<(Key, Value)>),
    TestLight(Zone),
    CaptureProfile(String, Option<String>),
    Preview(Option<Dpi>),
    Close,
}

pub enum Reply {
    Scan(Result<(Vec<Device>, Option<Device>, Status)>),
    Open(Result<Snapshot>),
    Apply(Result<Snapshot>),
    LightTest(Result<Snapshot>),
    ProfileCaptured(Result<Profile>),
    Preview(Result<()>),
    Close(Result<()>),
}

struct Preview {
    original: Dpi,
    latest: Dpi,
}

pub(crate) struct Session {
    backend: OpenRazer,
    current: Option<(Snapshot, Methods)>,
    preview: Option<Preview>,
}

impl Session {
    pub(crate) fn connect() -> Result<Self> {
        Ok(Self {
            backend: OpenRazer::connect()?,
            current: None,
            preview: None,
        })
    }

    pub(crate) fn scan(&mut self) -> Result<(Vec<Device>, Option<Device>, Status)> {
        let (devices, restarted) = self.backend.scan()?;
        if let Some((snapshot, _)) = &self.current {
            if !devices.contains(&snapshot.device) {
                self.current = None;
                self.preview = None;
            } else if restarted {
                self.revert()?;
                self.current = None;
            }
        }
        let Some((snapshot, methods)) = &self.current else {
            return Ok((devices, None, Status::default()));
        };
        let status = self.backend.status(&snapshot.device, methods);
        Ok((devices, Some(snapshot.device.clone()), status))
    }

    fn snapshot(&self) -> Result<&Snapshot> {
        self.current
            .as_ref()
            .map(|(snapshot, _)| snapshot)
            .ok_or_else(|| anyhow::anyhow!("Kein Gerät ausgewählt"))
    }

    fn settings(&mut self, device: Device) -> Result<Snapshot> {
        let (snapshot, methods) = self.backend.describe_fields(device)?;
        let visible = Snapshot {
            device: snapshot.device.clone(),
            fields: snapshot.fields.clone(),
            volatile_dpi: snapshot.volatile_dpi,
        };
        self.current = Some((snapshot, methods));
        Ok(visible)
    }

    pub(crate) fn open(&mut self, device: Device) -> Result<Snapshot> {
        self.revert()?;
        self.settings(device)
    }

    fn apply(&mut self, edits: Vec<(Key, Value)>) -> Result<Snapshot> {
        let _lock = profiles::lock("hardware")?;
        self.revert()?;
        let device = self.snapshot()?.device.clone();
        let (fresh, _) = self.backend.describe_fields(device.clone())?;
        profiles::backup(&fresh)?;
        self.backend.apply(&fresh, edits)?;
        profiles::mark_changed(&device);
        self.settings(device)
    }

    fn test_light(&mut self, zone: Zone) -> Result<Snapshot> {
        let _lock = profiles::lock("hardware")?;
        self.revert()?;
        let device = self.snapshot()?.device.clone();
        let (fresh, _) = self.backend.describe_fields(device.clone())?;
        let light = fresh
            .fields
            .iter()
            .find(|field| field.key == Key::Light(zone))
            .context("Für diese Zone ist kein Effekt verfügbar")?;
        let Some(brightness) = fresh
            .fields
            .iter()
            .find(|field| field.key == Key::Brightness(Some(zone)))
        else {
            bail!("Für den Test braucht die Zone eine eigene Helligkeitsregelung")
        };
        ensure!(
            matches!(&light.control, Control::Effects(effects) if effects.contains(&Effect::Static)),
            "Diese Zone unterstützt keine statische Farbe"
        );
        light
            .validates(&light.value)
            .context("Der aktuelle Effekt lässt sich nicht zuverlässig wiederherstellen")?;
        brightness.validates(&brightness.value)?;
        let turquoise = [0, 255, 180];
        let color = if matches!(light.value, Value::Light { effect: Some(Effect::Static), color, .. } if color == turquoise)
        {
            [255, 96, 0]
        } else {
            turquoise
        };
        let prior = vec![
            (brightness.key.clone(), brightness.value.clone()),
            (light.key.clone(), light.value.clone()),
        ];
        self.backend.apply(
            &fresh,
            vec![
                (brightness.key.clone(), Value::Number(65)),
                (
                    light.key.clone(),
                    Value::Light {
                        effect: Some(Effect::Static),
                        color,
                        direction: 1,
                    },
                ),
            ],
        )?;
        thread::sleep(Duration::from_secs(2));
        self.backend
            .apply(&fresh, prior)
            .context("LED-Test: Ausgangswerte konnten nicht wiederhergestellt werden")?;
        self.settings(device)
    }

    fn capture_profile(&mut self, name: &str, program: Option<&str>) -> Result<Profile> {
        let _lock = profiles::lock("hardware")?;
        self.revert()?;
        let device = self.snapshot()?.device.clone();
        let (snapshot, _) = self.backend.describe_fields(device)?;
        Profile::capture(name, program, &snapshot)
    }

    fn preview(&mut self, value: Option<Dpi>) -> Result<()> {
        let Some(value) = value else {
            return self.revert();
        };
        let snapshot = self.snapshot()?;
        if !snapshot.volatile_dpi {
            return Ok(());
        }
        let device = snapshot.device.clone();
        let actual = self.backend.dpi(&device)?;
        let original = self
            .preview
            .as_ref()
            .filter(|preview| preview.latest == actual)
            .map_or(actual, |preview| preview.original);
        if actual != value {
            let outcome = Hid::open(&device)?.preview_dpi(value);
            let observed = self.backend.dpi(&device);
            if observed.as_ref().map(|dpi| *dpi == value).unwrap_or(true) {
                self.preview = Some(Preview {
                    original,
                    latest: value,
                });
            }
            outcome?;
            ensure!(
                observed? == value,
                "Die Maus meldet eine andere Vorschau-DPI"
            );
        }
        self.preview = Some(Preview {
            original,
            latest: value,
        });
        Ok(())
    }

    fn revert(&mut self) -> Result<()> {
        if let Some(preview) = &self.preview {
            let device = &self.snapshot()?.device;
            if preview.original != preview.latest && self.backend.dpi(device)? == preview.latest {
                Hid::open(device)?.preview_dpi(preview.original)?;
                ensure!(
                    self.backend.dpi(device)? == preview.original,
                    "DPI-Vorschau konnte nicht zurückgesetzt werden"
                );
            }
            self.preview = None;
        }
        Ok(())
    }
}

pub struct Worker {
    pub send: Sender<Request>,
    pub replies: Receiver<Reply>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        let (send, requests) = mpsc::channel();
        let (reply, replies) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut session = None;
            while let Ok(request) = requests.recv() {
                if session.is_none() {
                    match Session::connect() {
                        Ok(backend) => session = Some(backend),
                        Err(error) => {
                            let response = if matches!(request, Request::Close) {
                                Reply::Close(Ok(()))
                            } else {
                                Reply::Scan(Err(error))
                            };
                            let _ = reply.send(response);
                            repaint();
                            if matches!(request, Request::Close) {
                                break;
                            }
                            continue;
                        }
                    }
                }
                let session = session.as_mut().expect("session initialized above");
                let response = match request {
                    Request::Scan => Reply::Scan(session.scan()),
                    Request::Open(device) => Reply::Open(session.open(device)),
                    Request::Apply(edits) => Reply::Apply(session.apply(edits)),
                    Request::TestLight(zone) => Reply::LightTest(session.test_light(zone)),
                    Request::CaptureProfile(name, program) => {
                        Reply::ProfileCaptured(session.capture_profile(&name, program.as_deref()))
                    }
                    Request::Preview(value) => Reply::Preview(session.preview(value)),
                    Request::Close => Reply::Close(session.revert()),
                };
                let closed = matches!(response, Reply::Close(Ok(())));
                if reply.send(response).is_err() {
                    let _ = session.revert();
                    break;
                }
                repaint();
                if closed {
                    break;
                }
            }
        });
        Self {
            send,
            replies,
            handle: Some(handle),
        }
    }

    pub fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(PartialEq, Eq, Serialize)]
pub struct TrayProfile {
    pub(crate) id: String,
    pub(crate) name: String,
}

fn tray_device(backend: &mut OpenRazer, serial: &str, pid: u16) -> Result<Device> {
    backend
        .scan()?
        .0
        .into_iter()
        .find(|device| device.serial == serial && device.pid == pid)
        .context("Das im Tray gewählte Gerät ist nicht erreichbar")
}

pub fn tray_profiles(serial: &str, pid: u16) -> Result<Vec<TrayProfile>> {
    let list = profiles::list()?;
    if list.is_empty() {
        return Ok(Vec::new());
    }
    let mut backend = OpenRazer::connect()?;
    let device = tray_device(&mut backend, serial, pid)?;
    Ok(profiles_for_device(&list, &device))
}

pub(crate) fn profiles_for_device(list: &[Profile], device: &Device) -> Vec<TrayProfile> {
    let model = profiles::model(device);
    list.iter()
        .filter(|profile| profile.model == model)
        .map(|profile| TrayProfile {
            id: profile.id(),
            name: profile.name.clone(),
        })
        .collect()
}

pub fn apply_tray_profile(serial: &str, pid: u16, profile_id: &str) -> Result<String> {
    let profile = profiles::list()?
        .into_iter()
        .find(|profile| profile.id() == profile_id)
        .context("Profil wurde entfernt")?;
    let mut session = Session::connect()?;
    let device = tray_device(&mut session.backend, serial, pid)?;
    let _lock = profiles::lock("hardware")?;
    let snapshot = session.open(device)?;
    let plan = profile.plan(&snapshot)?;
    if plan.changes.is_empty() {
        ensure!(
            plan.skipped.is_empty(),
            "Keine passende Änderung · {} Werte nicht verfügbar",
            plan.skipped.len()
        );
        return Ok(format!("{} · bereits aktiv", profile.name));
    }
    profiles::backup(&snapshot)?;
    session.backend.apply(&snapshot, plan.changes)?;
    profiles::mark_changed(&snapshot.device);
    Ok(if plan.skipped.is_empty() {
        format!("{} · übernommen", profile.name)
    } else {
        format!(
            "{} · übernommen, {} Werte nicht verfügbar",
            profile.name,
            plan.skipped.len()
        )
    })
}

pub fn automatic_tick() -> Result<Vec<String>> {
    automatic_tick_with_profiles(&profiles::list()?)
}

pub(crate) fn automatic_tick_with_profiles(profiles: &[Profile]) -> Result<Vec<String>> {
    if !profiles.iter().any(|profile| profile.program.is_some()) && !AutomaticRun::pending() {
        return Ok(Vec::new());
    }
    let programs = crate::profiles::running_programs()?;
    let _lock = crate::profiles::lock("automatic")?;
    let mut backend = OpenRazer::connect()?;
    let devices = backend.scan()?.0;
    let mut messages = Vec::new();
    for device in devices {
        match automatic_device(&mut backend, device, profiles, &programs) {
            Ok(Some(message)) => messages.push(message),
            Ok(None) => {}
            Err(error) => messages.push(format!("Spielprofile · {error:#}")),
        }
    }
    Ok(messages)
}

fn automatic_device(
    backend: &mut OpenRazer,
    device: Device,
    profiles: &[Profile],
    programs: &std::collections::BTreeSet<String>,
) -> Result<Option<String>> {
    let active = AutomaticRun::load(&device)?;
    let still_running = active.as_ref().is_some_and(|active| {
        programs.contains(&active.program)
            && profiles
                .iter()
                .any(|p| p.id() == active.profile_id && p.program.as_ref() == Some(&active.program))
    });
    if let Some(active) = &active
        && active.phase != AutoPhase::Prepared
        && still_running
    {
        return Ok(None);
    }
    if active
        .as_ref()
        .is_some_and(|active| active.phase == AutoPhase::Failed)
    {
        AutomaticRun::remove(&device)?;
        return Ok(None);
    }
    let model = profiles::model(&device);
    let selected = profiles.iter().find(|p| {
        p.model == model
            && p.program
                .as_ref()
                .is_some_and(|program| programs.contains(program))
    });
    if active.is_none() && selected.is_none() {
        return Ok(None);
    }
    let _lock = profiles::lock("hardware")?;
    let (snapshot, _) = backend.describe_fields(device.clone())?;
    if let Some(mut active) = active {
        let plan = active.restore(&snapshot).plan(&snapshot)?;
        let restored = plan.changes.len();
        if !plan.changes.is_empty() {
            backend.apply(&snapshot, plan.changes)?;
            crate::profiles::mark_changed(&device);
        }
        if still_running {
            active.phase = AutoPhase::Failed;
            active.save(&device)?;
            return Ok(Some(
                "Abgebrochenen Spielprofilwechsel zurückgesetzt".into(),
            ));
        }
        AutomaticRun::remove(&device)?;
        return Ok((restored > 0)
            .then(|| format!("Spielprofil beendet · {restored} Werte wiederhergestellt")));
    }
    let Some(profile) = selected else {
        return Ok(None);
    };
    let mut active = AutomaticRun {
        profile_id: profile.id(),
        program: profile.program.clone().unwrap_or_default(),
        prior: Profile::capture("Vor dem Spiel", None, &snapshot)?,
        applied: profile.clone(),
        phase: AutoPhase::Prepared,
    };
    let plan = profile.plan(&snapshot)?;
    active.record_change(&plan)?;
    active.save(&device)?;
    if !plan.changes.is_empty() {
        backend.apply(&snapshot, plan.changes)?;
        crate::profiles::mark_changed(&device);
    }
    active.phase = AutoPhase::Applied;
    active.save(&device)?;
    Ok(Some(format!("Spielprofil {} übernommen", profile.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect() -> Session {
        Session {
            backend: OpenRazer::connect().unwrap(),
            current: None,
            preview: None,
        }
    }

    fn polling(session: &Session) -> Result<i32> {
        let (snapshot, methods) = session.current.as_ref().unwrap();
        session
            .backend
            .status(&snapshot.device, methods)
            .polling
            .ok_or_else(|| anyhow::anyhow!("Keine Polling-Rückmeldung"))
    }

    #[test]
    #[ignore = "requires a private OpenRazer simulator and temporary XDG_CONFIG_HOME"]
    fn basilisk_fake_profiles_work_across_connections() -> Result<()> {
        ensure!(std::env::var("POINTRILL_FAKE_SESSION").as_deref() == Ok("1"));
        let mut session = connect();
        let (devices, _, _) = session.scan()?;
        let wired = devices
            .iter()
            .find(|device| device.vid == 0x1532 && device.pid == 0xe2)
            .context("Basilisk-Kabelfixture fehlt")?;
        let receiver = devices
            .iter()
            .find(|device| device.vid == 0x1532 && device.pid == 0xe3)
            .context("Basilisk-Dongle-Fixture fehlt")?;
        let zone = crate::model::ZONES
            .into_iter()
            .find(|zone| zone.id == "backlight")
            .unwrap();
        let brightness = Key::Brightness(Some(zone));
        let effect = Key::Light(zone);
        let colored = Value::Light {
            effect: Some(Effect::Static),
            color: [255, 91, 0],
            direction: 1,
        };
        let value = |snapshot: &Snapshot, key: &Key| {
            snapshot
                .fields
                .iter()
                .find(|field| &field.key == key)
                .unwrap()
                .value
                .clone()
        };
        session.open(wired.clone())?;
        let source = session.apply(vec![
            (brightness.clone(), Value::Number(63)),
            (effect.clone(), colored.clone()),
        ])?;
        let profile = Profile::capture("Training", Some("pointrill-test-game"), &source)?;
        profiles::save(&profile, false)?;
        let export = std::path::PathBuf::from(
            std::env::var_os("XDG_CONFIG_HOME").context("Test-Konfiguration fehlt")?,
        )
        .join("export.json");
        profiles::export(&profile, &export)?;
        assert!(profiles::import(&export)?.program.is_none());
        let before = session.open(receiver.clone())?;
        let candidates = tray_profiles(&receiver.serial, receiver.pid)?;
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.id == profile.id())
        );
        apply_tray_profile(&receiver.serial, receiver.pid, &profile.id())?;
        let updated = session.open(receiver.clone())?;
        assert_eq!(value(&updated, &brightness), Value::Number(63));
        assert_eq!(value(&updated, &effect), colored);
        let tested = session.test_light(zone)?;
        assert_eq!(value(&tested, &brightness), Value::Number(63));
        assert_eq!(value(&tested, &effect), colored);
        let previous = profiles::last(receiver)?;
        let plan = previous.plan(&tested)?;
        let restored = session.apply(plan.changes)?;
        assert_eq!(value(&restored, &brightness), value(&before, &brightness));
        assert_eq!(value(&restored, &effect), value(&before, &effect));

        let profiles = [profile];
        let mut running = std::collections::BTreeSet::from(["pointrill-test-game".into()]);
        assert!(
            automatic_device(&mut session.backend, receiver.clone(), &profiles, &running)?
                .is_some()
        );
        assert!(
            automatic_device(&mut session.backend, receiver.clone(), &profiles, &running)?
                .is_none()
        );
        let active = session.open(receiver.clone())?;
        assert_eq!(value(&active, &brightness), Value::Number(63));
        session.apply(vec![(brightness.clone(), Value::Number(47))])?;
        running.clear();
        assert!(
            automatic_device(&mut session.backend, receiver.clone(), &profiles, &running)?
                .is_some()
        );
        let restored = session.open(receiver.clone())?;
        assert_eq!(value(&restored, &brightness), Value::Number(47));
        assert_eq!(value(&restored, &effect), value(&before, &effect));
        assert!(AutomaticRun::load(receiver)?.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an isolated OpenRazer fake-device D-Bus session"]
    fn viper_fake_receiver_can_apply_polling() {
        assert_eq!(std::env::var("POINTRILL_FAKE_SESSION").as_deref(), Ok("1"));
        let mut session = connect();
        let (devices, _, _) = session.scan().unwrap();
        assert!(devices.iter().any(|device| device.pid == 0xe5));
        let receiver = devices.into_iter().find(Device::viper_receiver).unwrap();
        let snapshot = session.open(receiver).unwrap();
        assert!(snapshot.fields.iter().any(|field| field.key == Key::Stages));
        assert!(!snapshot.volatile_dpi);
        let original = polling(&session).unwrap();
        let trial = if original == 1000 { 500 } else { 1000 };
        session
            .apply(vec![(Key::Polling, Value::Number(trial))])
            .unwrap();
        assert_eq!(polling(&session).unwrap(), trial);
        session
            .apply(vec![(Key::Polling, Value::Number(original))])
            .unwrap();
        assert_eq!(polling(&session).unwrap(), original);
    }

    #[test]
    #[ignore = "temporarily changes one LED on an attached Viper V4 Pro receiver"]
    fn viper_hardware_restores_dongle_leds() -> Result<()> {
        ensure!(
            std::env::var("POINTRILL_REAL_HARDWARE").as_deref() == Ok("1"),
            "Hardwaretest braucht explizite Freigabe"
        );
        let mut session = connect();
        let (devices, _, _) = session.scan()?;
        let mut receivers = devices.into_iter().filter(Device::viper_receiver);
        let device = receivers
            .next()
            .ok_or_else(|| anyhow::anyhow!("Kein Viper-V4-Pro-Empfänger erkannt"))?;
        ensure!(
            receivers.next().is_none(),
            "Mehr als ein Viper-Empfänger erkannt"
        );
        let snapshot = session.open(device.clone())?;
        let field = snapshot
            .fields
            .iter()
            .find(|field| matches!(field.key, Key::Indicators | Key::ViperIndicators))
            .ok_or_else(|| anyhow::anyhow!("Dongle-LED-Einstellungen fehlen"))?;
        let Value::Indicators(original) = field.value else {
            anyhow::bail!("Drei LED-Modi erwartet");
        };
        let mut trial = original;
        trial[0] = i32::from(original[0] == 0);
        let observed = session
            .apply(vec![(field.key.clone(), Value::Indicators(trial))])
            .and_then(|_| Hid::open(&device)?.indicators());
        session.apply(vec![(field.key.clone(), Value::Indicators(original))])?;
        ensure!(
            Hid::open(&device)?.indicators()? == original,
            "Ausgangswerte der Dongle-LEDs wurden nicht wiederhergestellt"
        );
        ensure!(
            observed? == trial,
            "Der Dongle meldet einen anderen LED-Modus"
        );
        println!("Viper-Dongle · LED-Modi geprüft und wiederhergestellt: {original:?}");
        Ok(())
    }

    #[test]
    #[ignore = "temporarily exercises an attached Basilisk V4 Pro"]
    fn basilisk_hardware_restores_dpi_preview_and_polling() -> Result<()> {
        ensure!(
            std::env::var("POINTRILL_REAL_HARDWARE").as_deref() == Ok("1"),
            "Hardwaretest braucht explizite Freigabe"
        );
        let mut session = connect();
        let (devices, _, _) = session.scan()?;
        let device = devices
            .into_iter()
            .find(Device::basilisk)
            .ok_or_else(|| anyhow::anyhow!("Keine Basilisk V4 Pro"))?;
        let snapshot = session.open(device.clone())?;
        ensure!(snapshot.volatile_dpi, "HID-Zugriff fehlt");
        let original = session.backend.dpi(&device)?;
        let original_poll = polling(&session)?;
        let stages = snapshot
            .fields
            .iter()
            .find(|field| field.key == Key::Stages)
            .map(|field| field.value.clone());
        let candidate = if original.0 > 150 {
            (original.0 - 50, original.1 - 50)
        } else {
            (original.0 + 50, original.1 + 50)
        };
        let preview: Result<()> = (|| {
            session.preview(Some(candidate))?;
            ensure!(
                session.backend.dpi(&device)? == candidate,
                "DPI-Vorschau wurde nicht übernommen"
            );
            let (live, _) = session.backend.describe_fields(device.clone())?;
            ensure!(
                live.fields
                    .iter()
                    .find(|field| field.key == Key::Stages)
                    .map(|field| field.value.clone())
                    == stages,
                "Die gespeicherten Stufen haben sich während der Vorschau verändert"
            );
            Ok(())
        })();
        session.revert()?;
        preview?;
        ensure!(
            session.backend.dpi(&device)? == original,
            "Ausgangs-DPI wurde nicht wiederhergestellt"
        );
        let trial = if original_poll == 1000 { 500 } else { 1000 };
        let applied = session.apply(vec![(Key::Polling, Value::Number(trial))]);
        let reached = polling(&session);
        session.apply(vec![(Key::Polling, Value::Number(original_poll))])?;
        applied?;
        ensure!(
            reached? == trial && polling(&session)? == original_poll,
            "Pollingwechsel oder Rückkehr fehlgeschlagen"
        );
        println!(
            "Basilisk · flüchtige DPI und gespeicherte Stufen geprüft · Polling {original_poll} Hz wiederhergestellt"
        );
        Ok(())
    }
}
