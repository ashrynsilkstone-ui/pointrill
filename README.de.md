# Pointrill

[English](README.md)

Pointrill ist eine native Rust-App für Mäuse und Empfänger unter OpenRazer. Sie erkennt Geräte beim Start und beim Anschließen. Pro Gerät zeigt sie die Einstellungen an, die der jeweilige OpenRazer-Treiber bereitstellt.

Die Geräteübersicht zeigt das angeschlossene Modell, die Verbindungsart und die von OpenRazer bereitgestellten Live-Werte für Akku, DPI und Pollingrate. Der übrige Platz steht für die Geräteeinstellungen zur Verfügung.

- **Sensor** enthält DPI, DPI-Stufen und Pollingrate.
- **Beleuchtung** enthält die verfügbaren Effekte und Helligkeitsregler pro Zone.
- **Scrollrad** enthält die Radfunktionen des Geräts. Bei der Basilisk V4 Pro durchläuft die belegte Taste die gewählten Modi. Smart Reel und Scrollbeschleunigung stehen bei Kombinationen mit Freilauf zur Verfügung.
- **Akku und Dongle** zeigen Ladezustand, Ruhezustand, Akkuwarnung und unterstützte Status-LEDs.
- **Profile** speichert Einstellungen pro Mausmodell. Kabel und Empfänger können dieselbe Sammlung verwenden, wenn OpenRazer sie als dasselbe Modell erkennt.
- **Gerätetest** zeigt USB-Erkennung, OpenRazer-Status, Zugriff und die aktive DPI-Stufe. Der Diagnosebericht ist kopierbar.

Die Viper V4 Pro erscheint als Maus über USB (`1532:00E5`) oder mit ihrem HyperSpeed-Dongle (`1532:00E6`). Mit angeschlossenem Empfänger lassen sich im Reiter **Dongle** alle drei LEDs einzeln auf Aus, Akkustand, Verbindungsqualität, Pollingrate oder DPI-Stufe stellen. Pointrill verwendet dafür nach Möglichkeit OpenRazer und kann bei vorhandenem Zugriff auch die USB-Steuerschnittstelle des Empfängers nutzen. [OpenRazer-Geräteunterstützung](https://github.com/openrazer/openrazer/pull/2751) · [Messdaten zu den LED-Modi](https://github.com/openrazer/openrazer/issues/2760)

## Vorschau und Übernehmen

Änderungen erscheinen sofort im Fenster. Bei der Basilisk V4 Pro mit HID-Zugriff lässt sich auch die DPI direkt am Sensor vorführen. Beim Verwerfen oder normalen Schließen stellt die App den ursprünglichen DPI-Wert wieder her, sofern die Maus erreichbar ist und noch den Vorschauwert meldet.

Erst **Übernehmen** schreibt den Entwurf über die Speicherbefehle des Geräts oder über OpenRazer. Welche Werte nach dem Abziehen erhalten bleiben, hängt vom Gerät und Treiber ab. Pointrill liest die übernommenen Werte zurück und versucht bei einem Abbruch, bereits geänderte Werte wiederherzustellen.

## Profile und Gerätetest

Speichere unter **Profile** die aktuellen Geräteeinstellungen. **Änderungen ansehen** zeigt Ziel und Ausgangswert vor dem Übernehmen. **Letzten Stand wiederherstellen** lädt den Stand vor der letzten manuellen Änderung an der gewählten Verbindung in die Vorschau. JSON-Dateien lassen sich importieren und exportieren. Pointrill zeigt Werte an, die der gewählte Anschluss nicht unterstützt.

Im **Gerätetest** kannst du Tasten und Scrollrichtung live prüfen. Bei fehlendem Zugriff auf Linux-Eingabeereignisse kannst du Klicks und Scrollen im Testfeld prüfen. Unterstützte LED-Zonen leuchten beim Test zwei Sekunden und erhalten danach ihre vorherigen Werte.

Der [OpenRazer Mouse Tray](https://github.com/ashrynsilkstone-ui/basilisk-v4-tray) kann Profile für das gewählte Gerät übernehmen. Unter **Spielzuordnung** kannst du optional den Dateinamen eines Spiels eintragen, etwa `cs2`. Solange der Tray läuft, übernimmt er das Profil beim Start des Spiels. Beim Beenden stellt er die Werte wieder her, die seit dem automatischen Wechsel nicht manuell geändert wurden. Exportdateien enthalten die Mauskonfiguration. Spielzuordnungen werden in Pointrill eingerichtet.

## Start unter Linux

Zum Bauen brauchst du Rust ab Version 1.95. Für den Betrieb brauchst du eine Wayland- oder X11-Sitzung mit OpenGL und den OpenRazer-Treiber samt Daemon für deine Distribution. Starte OpenRazer in deiner Desktop-Sitzung und führe im Projektordner aus.

```sh
cargo run --locked --release
```

Für den Schnellwechsel im KDE-Tray installiere den gebauten Pointrill-Starter im Benutzerkonto.

```sh
cargo build --locked --release
install -Dm755 target/release/pointrill ~/.local/bin/pointrill
```

Die aktive DPI-Stufe und die Pollingrate wählst du über direkte Tasten. Klick auf einen DPI-Wert, um ihn einzugeben, oder ziehe ihn, um ihn zu ändern. Mit `cargo run --locked -- --probe` listest du erkannte Geräte und ihre verfügbaren Einstellungen auf.


Die flüchtige DPI-Vorschau und die zusätzlichen Radfunktionen der Basilisk V4 Pro sowie der LED-Zugriff für den Viper-V4-Pro-Dongle nutzen USB-Schnittstelle 03. Fehlt der Zugriff, installiere die optionale Regel und stecke das Gerät neu ein.

```sh
sudo install -Dm644 packaging/69-pointrill.rules /etc/udev/rules.d/69-pointrill.rules
sudo udevadm control --reload-rules
```

## Teststand

Privater Teststand. Die Basilisk V4 Pro wurde unter CachyOS über HyperSpeed geprüft. Bei der DPI-Vorschau und beim Verwerfen blieben die gespeicherten Stufen erhalten. Änderungen der Pollingrate erreichten die Maus erst bei **Übernehmen**. Danach wurde der Ausgangswert wiederhergestellt. Der OpenRazer-Simulator lieferte 123 Maus-Endpunkte, die Pointrill alle erkannte. Dazu gehörten die Viper V4 Pro per Kabel und per Empfänger. Beim Hardwaretest unter CachyOS wurden alle drei LED-Modi des Viper-V4-Pro-Dongles ausgelesen, ein Modus über **Übernehmen** geändert und die ursprünglichen Einstellungen wiederhergestellt.

Pointrill 0.3.0 wurde außerdem mit dem HyperSpeed-Empfänger der Basilisk V4 Pro (`1532:00E3`) und OpenRazer-Build `31453438` geprüft. Das Testprofil stellte die Pollingrate auf 500 Hz und den Unterboden auf statisch Orange bei 53 %. Die automatische Sicherung stellte 1000 Hz, den Effekt **Aus** und den ursprünglichen Helligkeitswert von 1 % wieder her. Pollingrate und Helligkeit wurden aus dem Treiber ausgelesen, der Effekt wurde über OpenRazer geprüft. Die Smart-Reel-Empfindlichkeit blieb auf Stufe 3. Pointrill meldete sie beim Wiederherstellen als übersprungen.

```sh
cargo test --locked
```
