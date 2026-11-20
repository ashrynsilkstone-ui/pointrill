import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from pointrill.backend import Backend, Binding, Device, Field
from pointrill.hid import VIPER_RECEIVER, HidControl


class BackendTest(unittest.TestCase):
    def setUp(self):
        self.backend = Backend(Mock())
        self.mouse = SimpleNamespace(dpi=(1200, 1200))
        info = Device("mouse", (0x1532, 0x00E3), "Basilisk", "USB-Empfänger")
        self.backend.selected = info.key
        self.backend.devices = {info.key: info}
        self.backend.clients = {info.serial: self.mouse}
        self.backend.scan = Mock()

    def test_volatile_dpi_can_be_discarded(self):
        self.backend.hid = Mock()
        self.backend.hid.preview_dpi.side_effect = lambda dpi: setattr(self.mouse, "dpi", dpi)
        self.backend.preview_dpi(self.backend.selected, (1600, 1600))
        self.backend.preview_dpi(self.backend.selected, (2400, 2400))
        self.assertEqual(self.mouse.dpi, (2400, 2400))
        self.backend.clear_preview()
        self.assertEqual(self.mouse.dpi, (1200, 1200))

    def test_invalid_apply_is_rejected_before_any_write(self):
        change = Mock()
        field = Field("polling", "Pollingrate", "Sensor", "choice", 1000, ((1000, "1000 Hz"),))
        self.backend.bindings = {field.key: Binding(field, lambda: 1000, change)}
        with self.assertRaises(ValueError):
            self.backend.apply(self.backend.selected, {"polling": 777})
        change.assert_not_called()

    def test_partial_apply_restores_prior_values(self):
        values = {"warning": 15, "sleep": 300}
        for key in values:
            field = Field(key, key, "Akku", "number", values[key], minimum=0, maximum=900)
            self.backend.bindings[key] = Binding(field, lambda key=key: values[key],
                                                 lambda value, key=key: values.__setitem__(key, value))
        def rejected(_value):
            raise OSError("sleep rejected")
        self.backend.bindings["sleep"].write = rejected
        with self.assertRaisesRegex(OSError, "sleep rejected"):
            self.backend.apply(self.backend.selected, {"warning": 20, "sleep": 600})
        self.assertEqual(values, {"warning": 15, "sleep": 300})

    def test_viper_receiver_reports_use_documented_commands(self):
        control = HidControl("receiver", VIPER_RECEIVER)
        with patch.object(control, "open") as handle, patch.object(control, "report") as report:
            device = handle.return_value.__enter__.return_value
            report.return_value = b"\x01\x03\x04"
            self.assertEqual(control.indicators(), (1, 3, 4))
            report.assert_called_with(device, 7, 0x95, bytes(3))
            control.set_indicators((0, 2, 4))
            report.assert_called_with(device, 7, 0x15, b"\x00\x02\x04", b"\x00\x02\x04")

    def test_generic_receivers_do_not_use_viper_fallback(self):
        control = HidControl("mouse", (0x1532, 0x00B3))
        with self.assertRaises(OSError):
            control.set_indicators((1, 2, 3))
