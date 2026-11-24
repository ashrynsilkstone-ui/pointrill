# Pointrill

[Deutsch](README.de.md)

Pointrill is a native Rust desktop app for OpenRazer mice and their receivers on Linux. It detects devices at startup and as they connect. Each device gets the settings its OpenRazer driver exposes.

The device overview shows the connected model, connection type and live readings for the battery, DPI and polling rate when OpenRazer exposes them. The remaining space is used for device settings.

- **Sensor** covers DPI, DPI stages and polling rates.
- **Lighting** covers the available effects and brightness controls for each zone.
- **Scroll wheel** covers supported wheel settings. On a Basilisk V4 Pro, the assigned button cycles through the selected modes. Smart Reel and acceleration are available when the selection includes free spin.
- **Battery and receiver** cover charging status, the idle timeout, low-battery warning and supported receiver LEDs.
- **Profiles** store settings by mouse model. Wired and wireless connections share profiles when OpenRazer reports the same model.
- **Device test** shows USB detection, OpenRazer status, access and the active DPI stage. The diagnostic report can be copied.

The Viper V4 Pro is listed as a mouse over USB (`1532:00E5`) or through its HyperSpeed receiver (`1532:00E6`). With the receiver connected, the **Dongle** tab lets you set any of its three LEDs to off, battery level, connection quality, polling rate, or DPI stage. Pointrill uses OpenRazer's LED API when available and can also use the receiver's USB control interface if the desktop session has access. [OpenRazer device support](https://github.com/openrazer/openrazer/pull/2751) · [Receiver LED captures](https://github.com/openrazer/openrazer/issues/2760)

## Preview and apply

Edits appear immediately in the window. A Basilisk V4 Pro with HID access also receives a temporary DPI preview. Pointrill restores the previous DPI when the preview is discarded or the app closes normally, provided the device is still reachable and still reports the preview value.

Use **Übernehmen** (Apply) to write a draft through a persistence-capable command. A setting's persistence after unplugging depends on the device and the OpenRazer driver. Pointrill reads the result back and attempts to restore prior values if an apply fails partway through.

## Profiles and device tests

Save the mouse's current settings on the **Profile** page. Preview a profile to see the current and target values before applying. Restore the previous state to preview the last manual change for that connection. Profiles can be imported and exported as JSON. Pointrill reports values that the selected connection cannot apply.

The **Gerätetest** (Device test) page shows button presses and scroll directions. If Linux input events are inaccessible, use the test area for clicks and scrolling delivered to the window. Testing a supported LED zone lights it for two seconds and restores the prior values afterward.

The [OpenRazer Mouse Tray](https://github.com/ashrynsilkstone-ui/basilisk-v4-tray) can apply profiles to the selected mouse. Optionally associate a profile with a game by entering its executable name, such as `cs2`. While the tray is running, it applies the profile when the game starts. When the game exits, it restores values that have not been manually changed since the switch. JSON exports contain the mouse settings. Game associations are configured locally in Pointrill.

## Run on Linux

To build Pointrill, install Rust 1.95 or later. You also need a Wayland or X11 desktop with OpenGL and the OpenRazer driver and daemon for your distribution. Start OpenRazer in your desktop session, then run from the checkout.

```sh
cargo run --locked --release
```

Install the user executable to enable profile switching in the KDE tray.

```sh
cargo build --locked --release
install -Dm755 target/release/pointrill ~/.local/bin/pointrill
```

The active DPI stage and polling rate are direct buttons. Click a DPI number to type it, or drag the number to adjust it. Run `cargo run --locked -- --probe` to list the connected devices and their available settings.


Basilisk V4 Pro hardware DPI preview, its extra wheel controls and the Viper V4 Pro receiver LED fallback use USB interface 03. If the session lacks access, install the optional rule and reconnect the device.

```sh
sudo install -Dm644 packaging/69-pointrill.rules /etc/udev/rules.d/69-pointrill.rules
sudo udevadm control --reload-rules
```

## Test status

This is a private test project. The Basilisk V4 Pro was tested on CachyOS over HyperSpeed. The DPI preview and discard left the saved stages intact. Polling changes reached the mouse on Apply and were restored after testing. All 123 mouse endpoints in the OpenRazer simulator were discovered, including the wired and wireless Viper V4 Pro. A hardware test on CachyOS read all three Viper V4 Pro receiver LED modes, changed one through Apply, and restored the original settings.

Pointrill 0.3.0 was also tested on the Basilisk V4 Pro HyperSpeed receiver (`1532:00E3`) with OpenRazer build `31453438`. The test profile changed the polling rate to 500 Hz and set the underglow to static orange at 53%. Applying the automatic backup restored 1000 Hz, the off effect and the original brightness of 1%. Polling and brightness were read back from the driver, and the effect was checked through OpenRazer. Smart Reel sensitivity remained at level 3. Pointrill reported it as skipped during restoration.

```sh
cargo test --locked
```
