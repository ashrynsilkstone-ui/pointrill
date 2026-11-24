# Pointrill

[English](README.md)

Pointrill ist eine Linux-Anwendung für Mäuse und Empfänger, die OpenRazer unterstützt. Es erkennt angeschlossene Geräte und zeigt die jeweils verfügbaren Einstellungen.

## Funktionen

- DPI-Stufen, Pollingrate, Beleuchtungszonen, Scrollradmodi und unterstützte Empfänger-LEDs einstellen. Smart Reel und Scrollbeschleunigung setzen eine Modusauswahl mit Freilauf voraus.
- Akkustand und Ladestatus ansehen und, sofern verfügbar, Ruhezustand und Akkuwarnung einstellen.
- Änderungen als Vorschau prüfen und übernehmen, Profile speichern, frühere Einstellungen wiederherstellen und beim Spielstart automatisch das Profil wechseln.
- Mauseingaben testen und die empfangene Pollingrate sowie die CPU-Last live sehen.
- Das KDE-Tray zeigt DPI-Stufe und Akkubalken, wechselt Profile und bietet einen Schalter für den Autostart.

## Installation

Installiere und starte den OpenRazer-Treiber samt Daemon für dein Gerät. Lade das passende 64-Bit-Paket herunter. [Pointrill 0.4.3 herunterladen](https://github.com/ashrynsilkstone-ui/pointrill/releases/tag/v0.4.3)

| Distribution | Installationsbefehl |
| --- | --- |
| CachyOS | `sudo pacman -U ./pointrill-0.4.3-1-x86_64.pkg.tar.zst` |
| Fedora | `sudo dnf install ./pointrill-0.4.3-1.x86_64.rpm` |
| Debian und Ubuntu | `sudo apt install ./pointrill_0.4.3-1_amd64.deb` |

Öffne **Pointrill** im Anwendungsmenü. Das Tray startet bei der nächsten Anmeldung in KDE Plasma.

Einige Einstellungen der Basilisk V4 Pro und Viper V4 Pro benötigen direkten USB-Zugriff. Falls der Zugriff fehlt, installiere die optionale Regel und stecke das Gerät neu ein.

```sh
sudo install -Dm644 /usr/share/doc/pointrill/69-pointrill.rules /etc/udev/rules.d/69-pointrill.rules
sudo udevadm control --reload-rules
```

Aus dem Quellcode startest du Pointrill mit `cargo run --locked --release` und Rust ab Version 1.95.
