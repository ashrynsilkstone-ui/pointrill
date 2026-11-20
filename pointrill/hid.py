"""Device-specific features outside the current OpenRazer API."""

from contextlib import contextmanager
from fcntl import ioctl
from pathlib import Path
from struct import pack, unpack
from time import sleep


BASILISK = {(0x1532, 0x00E2), (0x1532, 0x00E3)}
VIPER_RECEIVER = (0x1532, 0x00E6)
CYCLES = {5: "24 und 48 Rastpunkte", 3: "24 und Freilauf",
          6: "48 und Freilauf", 7: "Alle drei Modi"}


class HidControl:
    def __init__(self, serial, ids):
        self.serial = serial
        self.ids = ids

    @contextmanager
    def open(self):
        vid, pid = self.ids
        if self.ids not in BASILISK and self.ids != VIPER_RECEIVER:
            raise OSError("Für dieses Gerät gibt es keine HID-Erweiterung.")
        pattern = f"*:1.3/0003:{vid:04X}:{pid:04X}.*"
        interfaces = Path("/sys/bus/usb/devices").glob(pattern)
        nodes = [node for interface in interfaces
                 if f"HID_UNIQ={self.serial}" in (interface / "uevent").read_text().splitlines()
                 for node in interface.glob("hidraw/hidraw*")]
        if len(nodes) != 1:
            raise OSError("Die Steuerschnittstelle konnte nicht eindeutig zugeordnet werden.")
        with (Path("/dev") / nodes[0].name).open("r+b", buffering=0) as device:
            detail = bytearray(8)
            ioctl(device, 0x80084803, detail)
            if unpack("=IHH", detail) != (3, vid, pid):
                raise OSError("Die USB-Steuerschnittstelle hat sich geändert.")
            yield device

    @staticmethod
    def report(device, group, command, arguments, prefix=b""):
        packet = bytearray(91)
        packet[2] = 0x1F
        packet[6:9] = bytes((len(arguments), group, command))
        packet[9:9 + len(arguments)] = arguments
        for byte in packet[3:89]:
            packet[89] ^= byte
        for backoff in (0, .08, .15, .23, .32):
            if backoff:
                sleep(backoff)
            if ioctl(device, 0xC05B4806, packet) not in (90, 91):
                continue
            for delay in (.012, .024, .05):
                sleep(delay)
                reply = bytearray(91)
                if ioctl(device, 0xC05B4807, reply) != len(reply):
                    continue
                if reply[1:3] != b"\x02\x1f" or reply[6:9] != packet[6:9]:
                    continue
                payload = bytes(reply[9:9 + reply[6]])
                if payload.startswith(prefix):
                    return payload
        raise OSError(f"Keine passende Geräteantwort auf {group:02x}:{command:02x}.")

    def profile(self, device):
        slot, = self.report(device, 5, 0x84, b"\x00")
        if not 1 <= slot <= 5:
            raise OSError("Die Maus meldet kein gültiges Profil.")
        return slot

    def read(self, command, slot, expected_profile):
        with self.open() as device:
            if self.profile(device) != expected_profile:
                raise OSError("Das Mausprofil wurde gewechselt. Lade das Gerät erneut.")
            return self.report(device, *command, bytes((slot, 0)), bytes((slot,)))[1]

    def write_profile(self, get_command, set_command, profile, value, mask=0xFF):
        with self.open() as device:
            if self.profile(device) != profile:
                raise OSError("Das Mausprofil wurde gewechselt. Lade das Gerät erneut.")
            values = {slot: self.report(device, *get_command, bytes((slot, 0)), bytes((slot,)))[1]
                      for slot in (profile, 0)}
            try:
                for slot, previous in values.items():
                    data = bytes((slot, (previous & ~mask) | value))
                    self.report(device, *set_command, data, data)
            except OSError:
                for slot, previous in values.items():
                    # Zero wheel bits are an unreadable value on this firmware.
                    if mask != 0xFF and not previous & mask:
                        continue
                    data = bytes((slot, previous))
                    try:
                        self.report(device, *set_command, data, data)
                    except OSError:
                        pass
                raise

    def preview_dpi(self, value):
        if self.ids not in BASILISK:
            raise OSError("Für dieses Gerät ist keine flüchtige DPI-Vorschau hinterlegt.")
        # Explicit storage bank zero bypasses the daemon's persistent setter.
        arguments = pack(">BHHBB", 0, *value, 0, 0)
        with self.open() as device:
            self.report(device, 4, 5, arguments, b"\x00")

    def indicators(self):
        if self.ids != VIPER_RECEIVER:
            raise OSError("Die Status-LEDs gehören zum Viper-V4-Pro-Empfänger.")
        # OpenRazer issue #2760 documents the MI_03 captures for these commands.
        with self.open() as device:
            return tuple(self.report(device, 7, 0x95, bytes(3)))

    def set_indicators(self, modes):
        if self.ids != VIPER_RECEIVER:
            raise OSError("Die Status-LEDs gehören zum Viper-V4-Pro-Empfänger.")
        data = bytes(modes)
        with self.open() as device:
            self.report(device, 7, 0x15, data, data)
