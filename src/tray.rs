use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};

use crate::instance::APP_ID;
use crate::model::{Device, Status};
use crate::worker::{self, Session, TrayProfile};

const TRAY_ID: &str = "io.github.ashrynsilkstone_ui.Pointrill.Tray";

#[derive(Clone)]
enum Action {
    Open(bool),
    Select(Device),
    Apply(Device, String),
    SetAutostart(bool),
    Quit,
}

#[derive(Default, PartialEq)]
struct View {
    devices: Vec<Device>,
    device: Option<Device>,
    status: Status,
    profiles: Vec<TrayProfile>,
    autostart: bool,
    daemon_available: bool,
    message: Option<String>,
}

impl View {
    fn heading(&self) -> &str {
        self.device.as_ref().map_or(
            if self.daemon_available {
                "Keine Maus erkannt"
            } else {
                "Warte auf OpenRazer"
            },
            |device| device.name.as_str(),
        )
    }

    fn readings(&self) -> Vec<String> {
        let mut values = Vec::new();
        if let Some(percent) = self.status.battery {
            let charging = if self.status.charging {
                " · lädt"
            } else {
                ""
            };
            values.push(format!("Akku {percent} %{charging}"));
        }
        if let Some(stage) = self.status.dpi_stage {
            values.push(format!("DPI-Stufe {stage}"));
        }
        if let Some((x, y)) = self.status.dpi {
            values.push(if x == y || y == 0 {
                format!("{x} DPI")
            } else {
                format!("{x} × {y} DPI")
            });
        }
        if let Some(rate) = self.status.polling {
            values.push(format!("{rate} Hz"));
        }
        values
    }
}

struct PointrillTray {
    view: View,
    sender: Sender<Action>,
}

impl ksni::Tray for PointrillTray {
    fn id(&self) -> String {
        APP_ID.into()
    }
    fn title(&self) -> String {
        "Pointrill".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }
    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.sender.send(Action::Open(false));
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        true
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 44]
            .into_iter()
            .map(|size| tray_icon(size, self.view.status.battery, self.view.status.dpi_stage))
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let values = self.view.readings();
        ksni::ToolTip {
            icon_name: "input-mouse".into(),
            title: format!(
                "Pointrill · {}",
                self.view
                    .heading()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            ),
            description: values.join(" · "),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = vec![
            action_item("Pointrill öffnen", "input-mouse", Action::Open(false)),
            action_item(
                "Polling-Test öffnen",
                "utilities-system-monitor",
                Action::Open(true),
            ),
            MenuItem::Separator,
            info_item(self.view.heading().to_owned()),
        ];
        if self.view.devices.len() > 1 {
            let submenu = self
                .view
                .devices
                .iter()
                .map(|device| {
                    let current = self.view.device.as_ref() == Some(device);
                    action_item(
                        format!("{} · {}", device.name, device.connection()),
                        if current {
                            "object-select-symbolic"
                        } else {
                            "input-mouse"
                        },
                        Action::Select(device.clone()),
                    )
                })
                .collect();
            menu.push(
                SubMenu {
                    label: "Maus wählen".into(),
                    submenu,
                    ..Default::default()
                }
                .into(),
            );
        }
        menu.extend(self.view.readings().into_iter().map(info_item));
        if let Some(device) = &self.view.device
            && !self.view.profiles.is_empty()
        {
            let submenu = self
                .view
                .profiles
                .iter()
                .map(|profile| {
                    action_item(
                        &profile.name,
                        "",
                        Action::Apply(device.clone(), profile.id.clone()),
                    )
                })
                .collect();
            menu.push(
                SubMenu {
                    label: "Profil übernehmen".into(),
                    submenu,
                    ..Default::default()
                }
                .into(),
            );
        }
        if let Some(message) = &self.view.message {
            menu.push(info_item(message.clone()));
        }
        menu.push(MenuItem::Separator);
        menu.push(
            CheckmarkItem {
                label: "Bei der KDE-Anmeldung starten".into(),
                checked: self.view.autostart,
                activate: Box::new(|tray: &mut PointrillTray| {
                    let _ = tray.sender.send(Action::SetAutostart(!tray.view.autostart));
                }),
                ..Default::default()
            }
            .into(),
        );
        menu.push(action_item(
            "Tray beenden",
            "application-exit",
            Action::Quit,
        ));
        menu
    }
}

fn info_item(label: String) -> MenuItem<PointrillTray> {
    StandardItem {
        label: label.replace('_', "__"),
        enabled: false,
        ..Default::default()
    }
    .into()
}

fn action_item(label: impl AsRef<str>, icon: &str, action: Action) -> MenuItem<PointrillTray> {
    StandardItem {
        label: label.as_ref().replace('_', "__"),
        icon_name: icon.into(),
        activate: Box::new(move |tray: &mut PointrillTray| {
            let _ = tray.sender.send(action.clone());
        }),
        ..Default::default()
    }
    .into()
}

const ICON_GRID: i32 = 22;
const GREEN: [u8; 4] = [255, 122, 241, 43];
const LOW_BATTERY: [u8; 4] = [255, 255, 130, 119];
const READOUT: [u8; 4] = [255, 233, 239, 234];

fn tray_icon(size: i32, battery: Option<u8>, dpi_stage: Option<u8>) -> ksni::Icon {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let track = [210, 100, 117, 105];
    let low_battery = battery.is_some_and(|level| level <= 15);
    let charge = if low_battery { LOW_BATTERY } else { GREEN };
    let compact_mark = battery.is_some() || dpi_stage.is_some();
    for row in 0..size {
        for column in 0..size {
            let x = (column as f32 + 0.5) / size as f32;
            let y = (row as f32 + 0.5) / size as f32;
            let (mark_x, mark_y) = if compact_mark {
                ((x - 0.24) / 0.54 + 0.5, (y - 0.43) / 0.67 + 0.43)
            } else {
                (x, y)
            };
            let stroke = ((mark_x - 0.5).abs() - (mark_y - 0.13) * 0.64).abs()
                < if compact_mark { 0.061 } else { 0.043 };
            let dot = (mark_x - 0.5).hypot(mark_y - 0.63) < 0.073;
            let pixel = if dot || ((0.13..=0.73).contains(&mark_y) && stroke) {
                GREEN
            } else if let Some(level) = battery
                && (0.81..0.92).contains(&y)
                && (0.17..0.83).contains(&x)
            {
                if x - 0.17 < 0.66 * f32::from(level.min(100)) / 100.0 {
                    charge
                } else {
                    track
                }
            } else {
                [0; 4]
            };
            data.extend_from_slice(&pixel);
        }
    }
    let mut icon = ksni::Icon {
        width: size,
        height: size,
        data,
    };
    if let Some(stage) = dpi_stage {
        let y = if battery.is_some() { 3 } else { 7 };
        let text = if stage < 100 {
            format!("D{stage}")
        } else {
            stage.to_string()
        };
        draw_readout(&mut icon, &text, y, GREEN);
    }
    if let Some(level) = battery {
        let level = level.min(100);
        let y = if dpi_stage.is_some() { 11 } else { 7 };
        let text = if level == 100 {
            "100".into()
        } else {
            format!("{level}%")
        };
        draw_readout(
            &mut icon,
            &text,
            y,
            if low_battery { LOW_BATTERY } else { READOUT },
        );
    }
    icon
}

fn draw_readout(icon: &mut ksni::Icon, text: &str, top: i32, color: [u8; 4]) {
    // A 3×5 pixel font keeps both readings legible at Plasma's small sizes.
    const NUMERALS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b111, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    let left = ICON_GRID - 4 * text.len() as i32;
    for (index, byte) in text.bytes().enumerate() {
        let glyph = match byte {
            b'0'..=b'9' => NUMERALS[usize::from(byte - b'0')],
            b'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
            b'%' => [0b101, 0b001, 0b010, 0b100, 0b101],
            _ => continue,
        };
        let offset = left + 4 * index as i32;
        for (row, bits) in glyph.into_iter().enumerate() {
            let y = top + row as i32;
            for column in 0..3 {
                if bits & (0b100 >> column) == 0 {
                    continue;
                }
                let x = offset + column;
                for py in y * icon.height / ICON_GRID..(y + 1) * icon.height / ICON_GRID {
                    for px in x * icon.width / ICON_GRID..(x + 1) * icon.width / ICON_GRID {
                        let start = (py * icon.width + px) as usize * 4;
                        icon.data[start..start + 4].copy_from_slice(&color);
                    }
                }
            }
        }
    }
}

pub fn run() -> Result<()> {
    let _ownership = match zbus::blocking::connection::Builder::session()?
        .name(TRAY_ID)?
        .replace_existing_names(false)
        .build()
    {
        Ok(connection) => connection,
        Err(zbus::Error::NameTaken) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let executable = env::current_exe()?;
    let (sender, actions) = mpsc::channel();
    let handle = PointrillTray {
        view: View::default(),
        sender,
    }
    .assume_sni_available(true)
    .spawn()
    .context("KDE-Tray konnte nicht gestartet werden")?;
    let mut session = None;
    let mut preferred = None;
    let mut windows: Vec<Child> = Vec::new();
    let mut message = None;
    let mut last_auto = None;
    loop {
        if handle.is_closed() {
            break;
        }
        if last_auto.is_none_or(|time: Instant| time.elapsed() >= Duration::from_secs(3)) {
            match worker::automatic_tick() {
                Ok(messages) if !messages.is_empty() => message = messages.last().cloned(),
                Ok(_) => {}
                Err(error) => eprintln!("Spielprofile: {error:#}"),
            }
            last_auto = Some(Instant::now());
        }
        let view = refresh(&mut session, preferred.as_ref(), message.clone());
        if handle
            .update(|tray| {
                if tray.view != view {
                    tray.view = view;
                }
            })
            .is_none()
        {
            break;
        }

        windows.retain_mut(|child| matches!(child.try_wait(), Ok(None)));

        match actions.recv_timeout(Duration::from_secs(1)) {
            Ok(Action::Open(diagnostics)) => {
                let mut command = Command::new(&executable);
                if diagnostics {
                    command.arg("--diagnostics");
                }
                match command.spawn() {
                    Ok(child) => windows.push(child),
                    Err(error) => {
                        message = Some(format!("Pointrill konnte nicht geöffnet werden: {error}"))
                    }
                }
            }
            Ok(Action::Select(device)) => {
                preferred = Some(device);
                message = None;
            }
            Ok(Action::Apply(device, profile)) => {
                message = Some(
                    worker::apply_tray_profile(&device.serial, device.pid, &profile)
                        .unwrap_or_else(|error| format!("Profil: {error:#}")),
                );
            }
            Ok(Action::SetAutostart(enabled)) => {
                message = write_autostart(enabled, &executable)
                    .err()
                    .map(|error| format!("Autostart: {error:#}"));
            }
            Ok(Action::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    handle.shutdown().wait();
    Ok(())
}

fn refresh(
    session: &mut Option<Session>,
    preferred: Option<&Device>,
    message: Option<String>,
) -> View {
    let mut view = View {
        autostart: autostart_enabled().unwrap_or(false),
        message,
        ..Default::default()
    };
    if session.is_none() {
        *session = Session::connect().ok();
    }
    let Some(reader) = session else {
        return view;
    };
    let Ok((devices, current, status)) = reader.scan() else {
        *session = None;
        return view;
    };
    view.daemon_available = true;
    view.device = preferred
        .and_then(|preferred| devices.iter().find(|device| *device == preferred))
        .or_else(|| devices.first())
        .cloned();
    view.devices = devices;
    if let Some(device) = &view.device {
        view.status = if current.as_ref() == Some(device) {
            status
        } else if reader.open(device.clone()).is_ok() {
            reader
                .scan()
                .map(|(_, _, status)| status)
                .unwrap_or_default()
        } else {
            Status::default()
        };
        view.profiles = worker::device_profiles(device).unwrap_or_default();
    }
    view
}

fn autostart_path() -> Result<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".config"))
        })
        .context("Kein Benutzer-Konfigurationsordner gefunden")?;
    Ok(config.join("autostart").join(format!("{TRAY_ID}.desktop")))
}

fn autostart_enabled() -> Result<bool> {
    match fs::read_to_string(autostart_path()?) {
        Ok(contents) => Ok(!contents.lines().any(|line| line.trim() == "Hidden=true")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn change_autostart(enabled: bool) -> Result<()> {
    write_autostart(enabled, &env::current_exe()?)
}

fn write_autostart(enabled: bool, executable: &Path) -> Result<()> {
    let path = autostart_path()?;
    if !enabled {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    }
    let executable = executable
        .to_str()
        .context("Der Pointrill-Pfad ist kein UTF-8-Text")?;
    ensure!(
        !executable.chars().any(char::is_control),
        "Der Pointrill-Pfad enthält Steuerzeichen"
    );
    let arg = executable
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('`', "\\\\`")
        .replace('$', "\\\\$")
        .replace('%', "%%");
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Pointrill Tray\nComment=Live mouse status and profiles\nComment[de]=Live-Status und Profile der Maus\nExec=\"{arg}\" --tray\nIcon=input-mouse\nTerminal=false\nStartupNotify=false\nOnlyShowIn=KDE;\n"
    );
    let directory = path.parent().context("Kein Autostart-Ordner")?;
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(".{TRAY_ID}.{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(desktop.as_bytes())?;
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_stage_and_battery_update_separate_readouts() {
        let original = tray_icon(22, Some(80), Some(1));
        let stage = tray_icon(22, Some(80), Some(2));
        let battery = tray_icon(22, Some(10), Some(1));
        let split = 10 * ICON_GRID as usize * 4;

        assert_ne!(original.data[..split], stage.data[..split]);
        assert_eq!(original.data[split..], stage.data[split..]);
        assert_eq!(original.data[..split], battery.data[..split]);
        assert_ne!(original.data[split..], battery.data[split..]);
    }
}
