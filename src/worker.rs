use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use anyhow::{Result, ensure};

use crate::{
    bus::{Methods, OpenRazer},
    hid::Hid,
    model::{Device, Dpi, Key, Snapshot, Status, Value},
};

pub enum Request {
    Scan,
    Open(Device),
    Apply(Vec<(Key, Value)>),
    Preview(Option<Dpi>),
    Close,
}

pub enum Reply {
    Scan(Result<(Vec<Device>, Option<Device>, Status)>),
    Open(Result<Snapshot>),
    Apply(Result<Snapshot>),
    Preview(Result<()>),
    Close(Result<()>),
}

struct Preview {
    original: Dpi,
    latest: Dpi,
}

struct Session {
    backend: OpenRazer,
    current: Option<(Snapshot, Methods)>,
    preview: Option<Preview>,
}

impl Session {
    fn scan(&mut self) -> Result<(Vec<Device>, Option<Device>, Status)> {
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
            product_image: snapshot.product_image.clone(),
        };
        self.current = Some((snapshot, methods));
        Ok(visible)
    }

    fn open(&mut self, device: Device) -> Result<Snapshot> {
        self.revert()?;
        self.settings(device)
    }

    fn apply(&mut self, edits: Vec<(Key, Value)>) -> Result<Snapshot> {
        self.revert()?;
        let (snapshot, _) = self
            .current
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Kein Gerät ausgewählt"))?;
        let device = snapshot.device.clone();
        self.backend.apply(snapshot, edits)?;
        self.settings(device)
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
                    match OpenRazer::connect() {
                        Ok(backend) => {
                            session = Some(Session {
                                backend,
                                current: None,
                                preview: None,
                            })
                        }
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
