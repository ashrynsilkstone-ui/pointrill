# Pointrill

[Deutsch](README.de.md)

Pointrill is a Qt desktop app for OpenRazer mice and their receivers on Linux. It detects devices at startup and as they connect. Each device gets the settings its OpenRazer driver exposes.

![Pointrill's sensor tab on a Basilisk V4 Pro](docs/sensor.png)

- **Sensor** covers DPI, DPI stages and polling rates.
- **Lighting** covers the available effects and brightness controls for each zone.
- **Scroll wheel** covers supported wheel settings. On a Basilisk V4 Pro, the assigned button cycles through the selected modes. Smart Reel and acceleration are available when the selection includes free spin.
- **Battery and receiver** cover charging status, the idle timeout, low-battery warning and supported receiver LEDs.

The Viper V4 Pro is listed as a mouse over USB (`1532:00E5`) or through its HyperSpeed receiver (`1532:00E6`). The **Dongle** tab appears when Pointrill can read the receiver's LED modes. It uses OpenRazer's LED API when available, then tries the documented Viper V4 Pro receiver commands if the optional HID rule is installed. [OpenRazer device support](https://github.com/openrazer/openrazer/pull/2751) · [Receiver LED captures](https://github.com/openrazer/openrazer/issues/2760)

## Preview and apply

Edits appear immediately in the window. A Basilisk V4 Pro with HID access also receives a temporary DPI preview. Pointrill restores the previous DPI when the preview is discarded or the app closes normally, provided the device is still reachable and still reports the preview value.

**Übernehmen** (Apply) is the only action that writes the draft through a persistence-capable command. A setting's persistence after unplugging depends on the device and the OpenRazer driver. Pointrill reads the result back and attempts to restore prior values if an apply fails partway through.

## Run on Linux

Install the OpenRazer driver, daemon and Python client for your distribution, along with the Python D-Bus bindings and PySide6. Start OpenRazer in your desktop session, then run from the checkout.

```sh
python -m pointrill
```

The project also provides a `pointrill` entry point when installed with `python -m pip install .`. Use a virtual environment with system packages enabled when your distribution provides the OpenRazer and D-Bus Python modules.

Basilisk V4 Pro hardware DPI preview, its extra wheel controls and the Viper V4 Pro receiver LED fallback use USB interface 03. If the session lacks access, install the optional rule and reconnect the device.

```sh
sudo install -Dm644 packaging/69-pointrill.rules /etc/udev/rules.d/69-pointrill.rules
sudo udevadm control --reload-rules
```

## Test status

This is a private test project. The Basilisk V4 Pro was tested on CachyOS over HyperSpeed. The DPI preview and discard left the saved stages intact. Polling changes reached the mouse on Apply and were restored after testing. All 123 mouse endpoints in the OpenRazer simulator were discovered, including the wired and wireless Viper V4 Pro. The Viper V4 Pro receiver LEDs were tested with simulated HID responses and await a physical hardware test.

```sh
python -m unittest discover -s tests
```
