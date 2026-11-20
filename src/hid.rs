use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::model::{Device, Dpi};

pub struct Hid(File);

impl Hid {
    pub fn open(device: &Device) -> Result<Self> {
        ensure!(
            device.basilisk() || device.viper_receiver(),
            "Für dieses Gerät ist kein HID-Zugriff hinterlegt"
        );
        let prefix = format!("0003:{:04X}:{:04X}.", device.vid, device.pid);
        let identity = format!("HID_UNIQ={}", device.serial);
        let mut nodes = Vec::new();
        for interface in fs::read_dir("/sys/bus/usb/devices")? {
            let interface = interface?;
            if !interface.file_name().to_string_lossy().ends_with(":1.3") {
                continue;
            }
            for entry in fs::read_dir(interface.path())? {
                let entry = entry?;
                if !entry.file_name().to_string_lossy().starts_with(&prefix) {
                    continue;
                }
                let uevent = fs::read_to_string(entry.path().join("uevent"))?;
                if uevent.lines().any(|line| line == identity) {
                    let folder = entry.path().join("hidraw");
                    for raw in fs::read_dir(folder)? {
                        nodes.push(raw?.file_name());
                    }
                }
            }
        }
        ensure!(
            nodes.len() == 1,
            "USB-Steuerschnittstelle konnte nicht eindeutig zugeordnet werden"
        );
        let fd = OpenOptions::new()
            .read(true)
            .write(true)
            .open(Path::new("/dev").join(&nodes[0]))
            .context("Kein Zugriff auf die USB-Steuerschnittstelle")?;
        let mut details = [0u8; 8];
        ioctl(&fd, 0x8008_4803, &mut details)?;
        ensure!(
            u32::from_ne_bytes(details[0..4].try_into()?) == 3
                && u16::from_ne_bytes(details[4..6].try_into()?) == device.vid
                && u16::from_ne_bytes(details[6..8].try_into()?) == device.pid,
            "Die USB-Steuerschnittstelle hat sich geändert"
        );
        Ok(Self(fd))
    }

    fn report(&self, class: u8, cmd: u8, bytes: &[u8], prefix: &[u8]) -> Result<Vec<u8>> {
        let mut packet = feature_packet(class, cmd, bytes);
        for wait in [0, 65, 130, 210, 300] {
            if wait != 0 {
                sleep(Duration::from_millis(wait));
            }
            if !matches!(ioctl(&self.0, 0xc05b_4806, &mut packet)?, 90 | 91) {
                continue;
            }
            for delay in [12, 24, 50] {
                sleep(Duration::from_millis(delay));
                let mut response = [0u8; 91];
                if ioctl(&self.0, 0xc05b_4807, &mut response)? != 91
                    || response[1..3] != [2, 0x1f]
                    || response[6..9] != packet[6..9]
                {
                    continue;
                }
                let data = &response[9..9 + bytes.len()];
                if data.starts_with(prefix) {
                    return Ok(data.to_vec());
                }
            }
        }
        bail!("Keine passende Geräteantwort auf {class:02X}:{cmd:02X}")
    }

    pub fn profile(&self) -> Result<u8> {
        let slot = self.report(5, 0x84, &[0], &[])?[0];
        ensure!((1..=5).contains(&slot), "Kein gültiges Mausprofil");
        Ok(slot)
    }

    fn ensure_profile(&self, expected: u8) -> Result<()> {
        ensure!(
            self.profile()? == expected,
            "Das Mausprofil wurde gewechselt. Lade die Einstellungen neu"
        );
        Ok(())
    }

    fn profile_value(&self, class: u8, get: u8, bank: u8) -> Result<u8> {
        Ok(self.report(class, get, &[bank, 0], &[bank])?[1])
    }

    fn set_profile_value(&self, class: u8, set: u8, bank: u8, value: u8) -> Result<()> {
        let data = [bank, value];
        self.report(class, set, &data, &data)?;
        Ok(())
    }

    pub fn cycle(&self, profile: u8) -> Result<u8> {
        self.ensure_profile(profile)?;
        Ok(self.profile_value(0x15, 0x8b, 0)? & 7)
    }

    pub fn sensitivity(&self, profile: u8) -> Result<u8> {
        self.ensure_profile(profile)?;
        self.profile_value(2, 0xa3, 0)
    }

    pub fn change_profile(&self, profile: u8, cycle: bool, value: u8) -> Result<()> {
        self.ensure_profile(profile)?;
        let (class, get, set, mask) = if cycle {
            (0x15, 0x8b, 0x0b, 7)
        } else {
            (2, 0xa3, 0x23, 0xff)
        };
        let slots = [
            (profile, self.profile_value(class, get, profile)?),
            (0, self.profile_value(class, get, 0)?),
        ];
        let result = (|| {
            for (slot, old) in slots {
                self.set_profile_value(class, set, slot, (old & !mask) | value)?;
            }
            if !cycle {
                for (slot, _) in slots {
                    ensure!(
                        self.profile_value(class, get, slot)? == value,
                        "Gerät meldet eine andere Empfindlichkeit"
                    );
                }
            }
            Ok(())
        })();
        if result.is_err() {
            for (slot, old) in slots {
                if !cycle || old & mask != 0 {
                    let _ = self.set_profile_value(class, set, slot, old);
                }
            }
        }
        result
    }

    pub fn preview_dpi(&self, (x, y): Dpi) -> Result<()> {
        let [xh, xl] = x.to_be_bytes();
        let [yh, yl] = y.to_be_bytes();
        self.report(4, 5, &[0, xh, xl, yh, yl, 0, 0], &[0])?;
        Ok(())
    }

    pub fn indicators(&self) -> Result<[i32; 3]> {
        let data = self.report(7, 0x95, &[0; 3], &[])?;
        Ok([data[0].into(), data[1].into(), data[2].into()])
    }

    pub fn set_indicators(&self, values: [i32; 3]) -> Result<()> {
        let values = values.map(|n| n as u8);
        self.report(7, 0x15, &values, &values)?;
        Ok(())
    }
}

fn ioctl(fd: &File, request: libc::c_ulong, buffer: &mut [u8]) -> std::io::Result<i32> {
    // The descriptor is live for the whole call and the caller supplies exactly
    // the buffer size encoded by each HIDIOC request.
    let output = unsafe { libc::ioctl(fd.as_raw_fd(), request, buffer.as_mut_ptr()) };
    if output < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(output)
    }
}

fn feature_packet(class: u8, command: u8, bytes: &[u8]) -> [u8; 91] {
    let mut packet = [0u8; 91];
    packet[2] = 0x1f;
    packet[6..9].copy_from_slice(&[bytes.len() as u8, class, command]);
    packet[9..9 + bytes.len()].copy_from_slice(bytes);
    packet[89] = packet[3..89].iter().fold(0, |crc, byte| crc ^ byte);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_packet_crc_covers_the_payload() {
        let packet = feature_packet(7, 0x15, &[1, 2, 4]);
        assert_eq!(&packet[..12], &[0, 0, 0x1f, 0, 0, 0, 3, 7, 0x15, 1, 2, 4]);
        assert_eq!(packet[89], 3 ^ 7 ^ 0x15 ^ 1 ^ 2 ^ 4);
    }
}
