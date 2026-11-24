use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::mem::size_of;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::model::{Device, Snapshot};

struct UsbDevice {
    path: PathBuf,
    vid: u16,
    pid: u16,
    name: String,
}

fn read_hex(path: impl AsRef<Path>) -> Option<u16> {
    u16::from_str_radix(fs::read_to_string(path).ok()?.trim(), 16).ok()
}

fn usb_devices() -> Vec<UsbDevice> {
    let mut devices: Vec<_> = fs::read_dir("/sys/bus/usb/devices")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path().canonicalize().ok()?;
            let vid = read_hex(path.join("idVendor"))?;
            let pid = read_hex(path.join("idProduct"))?;
            if vid != 0x1532 {
                return None;
            }
            let name = fs::read_to_string(path.join("product")).unwrap_or_default();
            Some(UsbDevice {
                path,
                vid,
                pid,
                name: name.trim().to_owned(),
            })
        })
        .collect();
    devices.sort_by_key(|device| device.pid);
    devices
}

fn exact_usb(device: &Device) -> Option<PathBuf> {
    if device.serial.is_empty() {
        return None;
    }
    let interfaces: Vec<_> = usb_devices()
        .into_iter()
        .filter(|usb| usb.vid == device.vid && usb.pid == device.pid)
        .collect();
    let mut exact = interfaces.iter().filter(|usb| {
        fs::read_to_string(usb.path.join("serial"))
            .is_ok_and(|serial| serial.trim() == device.serial)
            || usb
                .path
                .read_dir()
                .into_iter()
                .flatten()
                .flatten()
                .any(|interface| {
                    interface
                        .path()
                        .read_dir()
                        .into_iter()
                        .flatten()
                        .flatten()
                        .any(|hid| {
                            hid.file_name().to_string_lossy().starts_with("0003:")
                                && fs::read_to_string(hid.path().join("device_serial"))
                                    .is_ok_and(|serial| serial.trim() == device.serial)
                        })
                })
    });
    if let Some(found) = exact.next() {
        return exact.next().is_none().then(|| found.path.clone());
    }
    // A receiver can report a different serial to OpenRazer than its USB or
    // input interfaces. Fall back to the USB path only for an unambiguous match.
    if let [usb] = interfaces.as_slice()
        && device.connection() != "Bluetooth"
    {
        return Some(usb.path.clone());
    }
    None
}

pub fn report(
    devices: &[Device],
    snapshot: Option<&Snapshot>,
    daemon_error: Option<&str>,
) -> String {
    let usb = usb_devices();
    let mut report = vec![format!("Pointrill {}", env!("CARGO_PKG_VERSION"))];
    report.push(format!(
        "Linux {}",
        fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_default()
            .trim()
    ));
    report.push(String::new());
    if usb.is_empty() {
        report.push(
            "USB · Kein Razer-Gerät sichtbar. Prüfe Kabel, Dongle oder Bluetooth-Verbindung."
                .into(),
        );
    } else {
        for usb in &usb {
            report.push(format!(
                "USB · {:04X}:{:04X} {}",
                usb.vid, usb.pid, usb.name
            ));
        }
    }
    if let Some(error) = daemon_error {
        report.push(format!("OpenRazer · {error}"));
        report
            .push("Prüfe in der Benutzersitzung: systemctl --user status openrazer-daemon".into());
    } else if devices.is_empty() {
        report.push("OpenRazer · Keine Maus oder kein Empfänger gemeldet.".into());
        if !usb.is_empty() {
            report.push("Prüfe die Geräteunterstützung im installierten Treiber und stecke das Gerät nach einem Treiberwechsel kurz ab und wieder an.".into());
        }
    } else {
        for device in devices {
            report.push(format!(
                "OpenRazer · {:04X}:{:04X} {} ({})",
                device.vid,
                device.pid,
                device.name,
                device.connection()
            ));
        }
    }
    if let Some(snapshot) = snapshot {
        let device = &snapshot.device;
        report.push(format!(
            "Pointrill · {} Einstellungen für {}",
            snapshot.fields.len(),
            device.name
        ));
        if snapshot.fields.is_empty() {
            report.push("Lade das Gerät neu und prüfe die Meldungen des OpenRazer-Daemons.".into());
        }
        if device.basilisk()
            && !snapshot.fields.iter().any(|field| {
                matches!(
                    field.key,
                    crate::model::Key::Dpi | crate::model::Key::Stages
                )
            })
        {
            report.push("Sensorwerte fehlen. Bewege die Maus und klicke auf „Neu laden“.".into());
        }
        if device.basilisk() || device.viper_receiver() {
            let access = crate::hid::Hid::open(device);
            report.push(match access {
                Ok(_) => "USB-Steuerschnittstelle · Zugriff verfügbar".into(),
                Err(error) => format!("USB-Steuerschnittstelle · {error:#}. Prüfe die optionale Pointrill-udev-Regel und verbinde die Maus erneut."),
            });
        }
    }
    report.join("\n")
}

pub(crate) struct InputNode {
    device: PathBuf,
    event: PathBuf,
}

impl InputNode {
    pub(crate) fn has_relative_motion(&self) -> bool {
        // Linux writes capability bitmaps with the least significant word last.
        fs::read_to_string(self.device.join("capabilities/rel"))
            .ok()
            .and_then(|bits| u64::from_str_radix(bits.split_whitespace().last()?, 16).ok())
            .is_some_and(|bits| bits & 3 == 3)
    }

    pub(crate) fn open(&self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.event)
    }
}

pub(crate) fn device_inputs(device: &Device) -> impl Iterator<Item = InputNode> + '_ {
    let usb = exact_usb(device);
    fs::read_dir("/sys/class/input")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(move |entry| {
            let id = entry.file_name();
            let suffix = id.to_str()?.strip_prefix("event")?;
            if suffix.is_empty() || !suffix.as_bytes().iter().all(u8::is_ascii_digit) {
                return None;
            }
            let input = entry.path().join("device");
            if read_hex(input.join("id/vendor")) != Some(device.vid)
                || read_hex(input.join("id/product")) != Some(device.pid)
            {
                return None;
            }
            let matches = fs::read_to_string(input.join("uniq"))
                .is_ok_and(|serial| !device.serial.is_empty() && serial.trim() == device.serial)
                || usb.as_ref().is_some_and(|path| {
                    input
                        .canonicalize()
                        .is_ok_and(|node| node.starts_with(path))
                });
            matches.then(|| InputNode {
                device: input,
                event: Path::new("/dev/input").join(id),
            })
        })
}

const EVENT_OFFSET: usize = size_of::<libc::timeval>();
pub(crate) const EVENT_SIZE: usize = EVENT_OFFSET + 8;

pub(crate) fn input_events(bytes: &[u8]) -> impl Iterator<Item = (u16, u16, i32)> + '_ {
    // Reads from evdev are aligned to complete input_event records.
    bytes.as_chunks::<EVENT_SIZE>().0.iter().map(|event| {
        let kind = u16::from_ne_bytes([event[EVENT_OFFSET], event[EVENT_OFFSET + 1]]);
        let code = u16::from_ne_bytes([event[EVENT_OFFSET + 2], event[EVENT_OFFSET + 3]]);
        let value = i32::from_ne_bytes(event[EVENT_OFFSET + 4..EVENT_SIZE].try_into().unwrap());
        (kind, code, value)
    })
}

pub struct InputMonitor {
    files: Vec<File>,
    pub events: VecDeque<String>,
    pub message: String,
    pub fallback: bool,
}

impl InputMonitor {
    pub fn start(device: &Device) -> Self {
        let mut files = Vec::new();
        let mut denied = false;
        for input in device_inputs(device) {
            match input.open() {
                Ok(file) => files.push(file),
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => denied = true,
                Err(_) => {}
            }
        }
        let fallback = files.is_empty();
        let message = if !fallback {
            format!(
                "Linux-Ereignisse von {} Eingabeschnittstelle(n).",
                files.len()
            )
        } else if denied {
            "Direktzugriff fehlt. Das Testfeld zeigt Klicks und Scrollen, die KDE ans Fenster liefert.".into()
        } else {
            "Keine passende Linux-Eingabeschnittstelle. Du kannst Klicks und Scrollen im Testfeld prüfen.".into()
        };
        Self {
            files,
            events: VecDeque::new(),
            message,
            fallback,
        }
    }

    pub fn record(&mut self, event: String) {
        self.events.push_front(event);
        self.events.truncate(16);
    }

    pub fn poll(&mut self) {
        let mut events = Vec::new();
        let mut disconnected = false;
        let mut buffer = [0; EVENT_SIZE * 32];
        self.files.retain_mut(|file| match file.read(&mut buffer) {
            Ok(0) => {
                disconnected = true;
                false
            }
            Ok(length) => {
                let mut wheel = [None, None];
                for (kind, code, value) in input_events(&buffer[..length]) {
                    match (kind, code, value) {
                        (0, 0, _) => {
                            for text in wheel.iter_mut().filter_map(Option::take) {
                                events.push(text);
                            }
                        }
                        (1, code, 1) => events.push(button_event(code)),
                        (2, 6 | 8 | 11 | 12, value) if value != 0 => {
                            let horizontal = matches!(code, 6 | 12);
                            wheel[usize::from(horizontal)] =
                                Some(scroll_event(horizontal, value > 0));
                        }
                        _ => {}
                    }
                }
                for text in wheel.into_iter().flatten() {
                    events.push(text);
                }
                true
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                true
            }
            Err(_) => {
                disconnected = true;
                false
            }
        });
        if disconnected && self.files.is_empty() {
            self.fallback = true;
            self.message =
                "Linux-Eingabeschnittstelle getrennt. Nutze das Testfeld oder starte den Test neu."
                    .into();
        }
        for event in events {
            self.record(event);
        }
    }
}

fn button_event(code: u16) -> String {
    let name = match code {
        272 => "Linke Maustaste",
        273 => "Rechte Maustaste",
        274 => "Mittlere Maustaste",
        275 => "Seitentaste 1",
        276 => "Seitentaste 2",
        277 => "Vorwärts",
        278 => "Zurück",
        _ => return format!("Taste · Linux-Code {code}"),
    };
    name.into()
}

pub fn scroll_event(horizontal: bool, positive: bool) -> String {
    match (horizontal, positive) {
        (false, true) => "Scrollrad · hoch",
        (false, false) => "Scrollrad · runter",
        (true, true) => "Scrollrad · rechts",
        (true, false) => "Scrollrad · links",
    }
    .into()
}
