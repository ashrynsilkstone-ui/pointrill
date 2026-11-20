"""OpenRazer discovery, capability mapping and device writes for Pointrill."""

from dataclasses import dataclass, replace
from functools import partial
import logging
from time import sleep
from typing import Callable

import dbus
from openrazer.client.device import RazerDeviceFactory

from .hid import BASILISK, CYCLES, VIPER_RECEIVER, HidControl


LOG = logging.getLogger(__name__)
DEVICE_ERRORS = (dbus.DBusException, OSError, RuntimeError, ValueError, AttributeError, IndexError, TypeError)
INDICATOR = ((0, "Aus"), (1, "An"), (2, "Akku"), (3, "Verbindung"), (4, "Kopplung"))
MULTI_INDICATOR = ((0, "Aus"), (1, "Akku"), (2, "Verbindung"), (3, "Pollingrate"), (4, "DPI"))
ASSISTS = ("smart_reel", "acceleration")
ZONES = (
    ("logo", "logo", "Logo"), ("scroll_wheel", "scroll", "Scrollrad"),
    ("backlight", "backlight", "Unterboden"), ("left", "left", "Links"),
    ("right", "right", "Rechts"), ("charging", "charging", "Laden"),
    ("fast_charging", "fast_charging", "Schnellladen"),
    ("fully_charged", "fully_charged", "Voll geladen"),
)
EFFECTS = (("none", "Aus"), ("static", "Statisch"), ("spectrum", "Farbwechsel"),
           ("wave", "Welle"), ("breath_single", "Atmen"), ("breath_random", "Atmen mit Farbwechsel"),
           ("breath_mono", "Einfarbig atmen"), ("blinking", "Blinken"),
           ("pulsate", "Pulsieren"), ("on", "An"), ("wheel", "Farbrad"))
COLOR_EFFECTS = {"static", "breath_single", "blinking", "pulsate"}
DIRECTION_EFFECTS = {"wave", "wheel"}


@dataclass(frozen=True)
class Device:
    serial: str
    ids: tuple[int, int]
    name: str
    connection: str

    @property
    def key(self):
        return self.serial, *self.ids


@dataclass(frozen=True)
class Field:
    key: str
    label: str
    page: str
    kind: str
    value: object
    choices: tuple = ()
    minimum: int = 0
    maximum: int = 0


@dataclass
class Binding:
    field: Field
    read: Callable
    write: Callable


@dataclass
class Preview:
    original: tuple[int, int]
    latest: tuple[int, int]


class Backend:
    def __init__(self, prefs):
        self.prefs = prefs
        self.bus = None
        self.owner = None
        self.clients = {}
        self.devices = {}
        self.bindings = {}
        self.selected = None
        self.hid = None
        self.preview = None

    def scan(self):
        if self.bus is None:
            self.bus = dbus.SessionBus()
        owner = str(self.bus.get_name_owner("org.razer"))
        if owner != self.owner:
            if self.preview is not None:
                try:
                    self.clear_preview()
                except DEVICE_ERRORS as error:
                    LOG.debug("DPI-Vorschau beim Daemonwechsel: %s", error)
            self.clients.clear()
            self.bindings.clear()
            self.selected = None
            self.hid = None
            self.preview = None
            self.owner = owner
        root = self.bus.get_object(owner, "/org/razer", introspect=False)
        serials = list(map(str, dbus.Interface(root, "razer.devices").getDevices(timeout=3)))
        for serial in set(self.clients) - set(serials):
            del self.clients[serial]
        devices = {}
        for serial in serials:
            try:
                mouse = self.clients.get(serial)
                if mouse is not None:
                    path = f"/org/razer/device/{serial}"
                    proxy = self.bus.get_object(owner, path, introspect=False)
                    ids = tuple(dbus.Interface(proxy, "razer.device.misc").getVidPid(timeout=3))
                    if ids != (mouse._vid, mouse._pid):
                        self.clients.pop(serial)
                if serial not in self.clients:
                    self.clients[serial] = RazerDeviceFactory.get_device(serial)
            except DEVICE_ERRORS as error:
                LOG.debug("Geräteerkennung: %s", error)
                continue
            mouse = self.clients[serial]
            name = mouse.name
            name_lower = name.lower()
            receiver = any(word in name_lower for word in ("wireless", "receiver", "dongle", "mouse dock", "charging dock"))
            if mouse.type != "mouse" and not (mouse.type in ("accessory", "dongle") and receiver):
                continue
            ids = (int(mouse._vid), int(mouse._pid))
            connection = "Bluetooth" if "bluetooth" in name_lower else "USB-Empfänger" if receiver else "USB"
            info = Device(serial, ids, name, connection)
            devices[info.key] = info
        self.devices = devices
        if self.selected not in devices:
            self.selected = None
            self.hid = None
            self.preview = None
            self.bindings.clear()
        return sorted(devices.values(), key=lambda info: (info.name.casefold(), info.key))

    @property
    def mouse(self):
        if self.selected is None:
            raise OSError("Wähle ein erreichbares Gerät.")
        return self.clients[self.selected[0]]

    def add(self, key, label, page, kind, read, write, *, choices=(), minimum=0, maximum=0):
        try:
            value = read()
            if kind == "stages" and not value[1]:
                raise ValueError("Das Gerät meldet keine DPI-Stufen.")
        except DEVICE_ERRORS as error:
            LOG.debug("%s nicht erreichbar: %s", key, error)
            return
        field = Field(key, label, page, kind, value, tuple(choices), minimum, maximum)
        self.bindings[key] = Binding(field, read, write)

    def attribute(self, target, key, label, page, kind, attribute, **options):
        self.add(key, label, page, kind, partial(getattr, target, attribute),
                 partial(setattr, target, attribute), **options)

    def select(self, key):
        self.clear_preview()
        if key not in self.devices:
            raise OSError("Das ausgewählte Gerät ist nicht mehr verbunden.")
        self.selected = key
        self.bindings.clear()
        self.hid = None
        mouse = self.mouse
        self.sensor(mouse)
        self.power(mouse)
        self.lights(mouse)
        self.wheel(mouse)
        self.receiver(mouse)
        return list(self.fields()), self.hid is not None

    def fields(self):
        return (binding.field for binding in self.bindings.values())

    def sensor(self, mouse):
        if mouse.has("dpi"):
            try:
                max_dpi = int(mouse.max_dpi)
                available = tuple(mouse.available_dpi) if mouse.has("available_dpi") else ()
            except DEVICE_ERRORS as error:
                LOG.debug("DPI-Bereich: %s", error)
            else:
                if mouse.has("dpi_stages"):
                    def read_stages():
                        active, stages = mouse.dpi_stages
                        return active, tuple(map(tuple, stages))

                    def write_stages(value):
                        active, stages = value
                        mouse.dpi_stages = active, list(stages)
                        mouse.dpi = stages[active - 1]

                    self.add("stages", "DPI-Stufen", "Sensor", "stages", read_stages, write_stages,
                             minimum=100, maximum=max_dpi)
                if "stages" not in self.bindings:
                    self.attribute(mouse, "dpi", "Empfindlichkeit", "Sensor", "dpi", "dpi",
                                   choices=((value, str(value)) for value in available), minimum=100, maximum=max_dpi)
        if mouse.has("poll_rate") and mouse.has("supported_poll_rates"):
            try:
                rates = sorted(set(mouse.supported_poll_rates))
            except DEVICE_ERRORS as error:
                LOG.debug("Pollingraten: %s", error)
            else:
                if rates:
                    self.attribute(mouse, "polling", "Pollingrate", "Sensor", "choice", "poll_rate",
                                   choices=((rate, f"{rate} Hz") for rate in rates))

    def power(self, mouse):
        if mouse.has("idle_time"):
            self.add("sleep", "Ruhezustand nach", "Akku", "number", mouse.get_idle_time, mouse.set_idle_time,
                     minimum=0, maximum=900)
        if mouse.has("low_battery_threshold"):
            self.add("warning", "Akkuwarnung", "Akku", "number", mouse.get_low_battery_threshold,
                     mouse.set_low_battery_threshold, minimum=5, maximum=25)

    def lights(self, mouse):
        if mouse.has("brightness"):
            self.attribute(mouse, "brightness", "Helligkeit", "Beleuchtung", "brightness", "brightness",
                           minimum=0, maximum=100)
        self.light(mouse, mouse.fx, "main", "Gerät", "lighting_")
        for attribute, name, label in ZONES:
            zone = getattr(mouse.fx.misc, attribute, None)
            if zone is not None:
                self.light(mouse, zone, name, label, f"lighting_{name}_")
                if mouse.has(f"lighting_{name}_brightness"):
                    self.attribute(zone, f"brightness:{name}", f"Helligkeit · {label}", "Beleuchtung",
                                   "brightness", "brightness", minimum=0, maximum=100)

    def light(self, mouse, target, key, label, prefix):
        effects = tuple((name, text) for name, text in EFFECTS
                        if mouse.has(prefix + name) and callable(getattr(target, name, None)))
        if not effects:
            return

        def read():
            effect = str(target.effect)
            color = tuple(target.colors[:3]) if effect in COLOR_EFFECTS else None
            direction = target.wave_dir if effect in DIRECTION_EFFECTS else None
            return effect, color, direction

        def write(value):
            effect, color, direction = value
            if effect in COLOR_EFFECTS:
                getattr(target, effect)(*color)
            elif effect in DIRECTION_EFFECTS:
                getattr(target, effect)(direction)
            else:
                getattr(target, effect)()

        self.add(f"light:{key}", label, "Beleuchtung", "light", read, write, choices=effects)

    def wheel(self, mouse):
        is_basilisk = self.selected[1:] in BASILISK
        if is_basilisk:
            self.basilisk(mouse)
        elif mouse.has("scroll_mode"):
            self.attribute(mouse, "wheel", "Radmodus", "Scrollrad", "choice", "scroll_mode",
                           choices=((0, "Rastend"), (1, "Freilauf")))
        if is_basilisk and "cycle" not in self.bindings:
            return
        for key, label, capability in (("smart_reel", "Smart Reel", "scroll_smart_reel"),
                                       ("acceleration", "Scrollbeschleunigung", "scroll_acceleration")):
            if mouse.has(capability):
                self.attribute(mouse, key, label, "Scrollrad", "bool", capability)

    def basilisk(self, mouse):
        control = HidControl(self.selected[0], self.selected[1:])
        try:
            with control.open() as handle:
                profile = control.profile(handle)
        except DEVICE_ERRORS as error:
            LOG.debug("Basilisk-HID: %s", error)
            return
        self.hid = control
        pref_key = f"cycle/{control.serial}/{control.ids[1]:04x}/{profile}"

        def read_cycle():
            cycle = control.read((0x15, 0x8B), 0, profile) & 7
            saved_cycle = self.prefs.value(pref_key, 0, type=int)
            return cycle if cycle in CYCLES else saved_cycle if saved_cycle in CYCLES else None

        def write_cycle(cycle):
            control.write_profile((0x15, 0x8B), (0x15, 0x0B), profile, cycle, mask=7)
            self.prefs.setValue(pref_key, cycle)
            if mouse.has("scroll_mode") and not cycle & (1 << mouse.scroll_mode):
                mode = next(mode for mode in (0, 2, 1) if cycle & (1 << mode))
                mouse.scroll_mode = mode
                if not cycle & (1 << mouse.scroll_mode):
                    raise OSError("Die Maus hat keinen freigegebenen Radmodus übernommen.")

        self.add("cycle", "Modi beim Umschalten", "Scrollrad", "choice", read_cycle, write_cycle,
                 choices=CYCLES.items())
        self.add("sensitivity", "Smart-Reel-Empfindlichkeit", "Scrollrad", "choice",
                 lambda: control.read((2, 0xA3), 0, profile),
                 lambda level: control.write_profile((2, 0xA3), (2, 0x23), profile, level),
                 choices=((level, f"Stufe {level + 1}") for level in range(5)))

    def receiver(self, mouse):
        if mouse.has("hyperpolling_indicator_led"):
            self.add("indicator", "Status-LED", "Dongle", "choice", mouse.get_hyperpolling_indicator_led,
                     mouse.set_hyperpolling_indicator_led, choices=INDICATOR)
        if mouse.has("hyperpolling_multi_indicator_led_modes"):
            self.add("indicators", "Status-LEDs", "Dongle", "indicators", mouse.get_hyperpolling_multi_indicator_led_modes,
                     lambda value: mouse.set_hyperpolling_multi_indicator_led_modes(*value), choices=MULTI_INDICATOR)
        elif self.selected[1:] == VIPER_RECEIVER:
            control = HidControl(self.selected[0], VIPER_RECEIVER)
            self.add("indicators", "Status-LEDs", "Dongle", "indicators",
                     control.indicators, control.set_indicators, choices=MULTI_INDICATOR)

    def status(self):
        if self.selected is None:
            return {}
        mouse = self.mouse
        data = {}
        for capability, names in (("battery", ("battery_level", "is_charging")),
                                  ("dpi", ("dpi",)), ("poll_rate", ("poll_rate",))):
            if mouse.has(capability):
                for name in names:
                    try:
                        data[name] = getattr(mouse, name)
                    except DEVICE_ERRORS as error:
                        LOG.debug("Status %s: %s", name, error)
        return data

    def preview_dpi(self, key, value):
        if key != self.selected or self.hid is None:
            return
        position = tuple(self.mouse.dpi)
        original = (self.preview.original if self.preview is not None and position == self.preview.latest
                    else position)
        candidate = Preview(original, value)
        if position != value:
            try:
                self.hid.preview_dpi(value)
                if tuple(self.mouse.dpi) != value:
                    raise OSError("Die Maus hat die flüchtige DPI-Stufe anders zurückgemeldet.")
            except DEVICE_ERRORS:
                # A timed-out write may already have reached the mouse.
                try:
                    if tuple(self.mouse.dpi) == value:
                        self.preview = candidate
                except DEVICE_ERRORS:
                    self.preview = candidate
                raise
        self.preview = candidate

    def clear_preview(self):
        if self.preview is None:
            return
        preview = self.preview
        if tuple(self.mouse.dpi) == preview.latest and preview.original != preview.latest:
            self.hid.preview_dpi(preview.original)
            if tuple(self.mouse.dpi) != preview.original:
                raise OSError("Die flüchtige DPI-Vorschau ließ sich nicht zurücksetzen.")
        self.preview = None

    def validate(self, field, value):
        if field.kind == "choice":
            if value not in dict(field.choices):
                raise ValueError(f"Ungültige Auswahl für {field.label}.")
        elif field.kind in ("number", "brightness"):
            if not field.minimum <= value <= field.maximum:
                raise ValueError(f"Wert liegt außerhalb des Bereichs für {field.label}.")
        elif field.kind == "bool":
            if not isinstance(value, bool):
                raise ValueError(f"Ungültiger Schalter für {field.label}.")
        elif field.kind in ("dpi", "stages"):
            if field.kind == "stages":
                active, points = value
                if not 1 <= active <= len(points) <= 5:
                    raise ValueError("Wähle eine gültige DPI-Stufe.")
            else:
                points = (value,)
            for x, y in points:
                x_only = field.kind == "dpi" and field.value[1] == 0
                if field.choices:
                    valid = x in dict(field.choices) and y == 0
                elif x_only:
                    valid = field.minimum <= x <= field.maximum and y == 0
                else:
                    valid = all(field.minimum <= axis <= field.maximum for axis in (x, y))
                if not valid:
                    raise ValueError("DPI liegt außerhalb des Gerätebereichs.")
        elif field.kind == "indicators":
            if len(value) != 3 or any(item not in dict(field.choices) for item in value):
                raise ValueError("Wähle eine Funktion pro Status-LED.")
        elif field.kind == "light":
            effect, color, direction = value
            if effect not in dict(field.choices):
                raise ValueError("Die Beleuchtungszone unterstützt diesen Effekt nicht.")
            if effect in COLOR_EFFECTS and (color is None or len(color) != 3 or any(not 0 <= i <= 255 for i in color)):
                raise ValueError("Wähle eine gültige RGB-Farbe.")
            if effect in DIRECTION_EFFECTS and direction not in (1, 2):
                raise ValueError("Wähle eine Richtung für den Effekt.")

    def apply(self, key, changes):
        self.scan()
        if key != self.selected or key not in self.devices:
            raise OSError("Das Gerät ist nicht mehr verbunden.")
        changes = dict(changes)
        if "cycle" in self.bindings and ({"cycle", "sensitivity", *ASSISTS} & changes.keys()):
            cycle = changes["cycle"] if "cycle" in changes else self.bindings["cycle"].read()
            if changes.get("cycle") == 5:
                changes.update({item: False for item in ASSISTS if item in self.bindings})
            if not (cycle is not None and cycle & 2) and (
                any(changes.get(item) for item in ASSISTS) or "sensitivity" in changes
            ):
                raise ValueError("Wähle zuerst eine Radkombination mit Freilauf.")
        for name, value in changes.items():
            if name not in self.bindings:
                raise ValueError("Die Einstellung ist auf diesem Gerät nicht verfügbar.")
            self.validate(self.bindings[name].field, value)
        self.clear_preview()
        ordered = sorted(changes, key=lambda name: (name != "cycle", name in ASSISTS))
        before = {name: self.bindings[name].read() for name in ordered}
        written = []
        values = dict(before)
        try:
            for name in ordered:
                binding = self.bindings[name]
                after = changes[name]
                if after == before[name]:
                    continue
                written.append(name)
                binding.write(after)
                if name == "polling":
                    # A receiver may briefly lose the mouse while changing the RF rate.
                    sleep(.65)
                actual = binding.read()
                values[name] = actual
                if actual != after and not (
                    binding.field.kind == "brightness" and abs(actual - after) <= 1
                ):
                    raise OSError(f"Die Maus meldet einen anderen Wert für {binding.field.label}.")
        except DEVICE_ERRORS as error:
            incomplete = False
            for name in reversed(written):
                binding = self.bindings[name]
                try:
                    if before[name] is None:
                        incomplete = True
                    elif binding.read() != before[name]:
                        binding.write(before[name])
                        incomplete |= binding.read() != before[name]
                except DEVICE_ERRORS:
                    incomplete = True
            if incomplete:
                raise OSError(f"{error} Einige Werte konnten nicht zurückgesetzt werden. Lade das Gerät erneut.") from error
            raise
        for name in ordered:
            binding = self.bindings[name]
            binding.field = replace(binding.field, value=values[name])
        return list(self.fields())
