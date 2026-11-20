"""Pointrill's Qt desktop application."""

from pathlib import Path
import logging
import sys

from PySide6.QtCore import QObject, QRectF, QSettings, QSize, Qt, QThread, QTimer, Signal, Slot
from PySide6.QtGui import QColor, QPainter, QPen, QPixmap
from PySide6.QtWidgets import (
    QApplication, QFrame, QHBoxLayout, QLabel, QListWidget, QListWidgetItem,
    QMainWindow, QMessageBox, QPushButton, QScrollArea, QSizePolicy, QTabWidget,
    QVBoxLayout, QWidget,
)

from .backend import ASSISTS, Backend, DEVICE_ERRORS
from .hid import BASILISK
from .widgets import Editor


STYLE = """
QMainWindow, QWidget#canvas { background: #14212d; color: #eef5fb; }
QWidget { color: #edf5fb; font-size: 14px; }
QLabel#brand { font-size: 23px; font-weight: 700; }
QLabel#subtitle, QLabel#muted { color: #aabed0; }
QLabel#eyebrow { font-size: 12px; color: #8ee8d1; font-weight: 600; }
QLabel#deviceTitle { font-size: 28px; font-weight: 700; }
QLabel#fieldTitle { font-weight: 600; }
QWidget#sidebar { background: #101b26; border-right: 1px solid #2c4052; }
QFrame#preview { background: #111f2b; border: 1px solid #344b5d; border-radius: 13px; }
QWidget#settingsPane, QWidget#settingsViewport { background: #14212d; }
QWidget#editor { background: #1c2c3b; border: 1px solid #2f4456; border-radius: 9px; }
QListWidget, QScrollArea, QTabWidget::pane { background: transparent; border: 0; }
QListWidget::item { padding: 10px 8px; margin: 2px 0; border-radius: 6px; }
QListWidget::item:selected { background: #24443f; color: #a9f2de; }
QTabBar::tab { background: #1d2d3c; padding: 8px 12px; margin-right: 4px; border-radius: 6px; color: #acbfd0; }
QTabBar::tab:selected { background: #274740; color: #a9f2de; }
QPushButton, QComboBox, QSpinBox { background: #263647; border: 1px solid #3c5369; border-radius: 6px; min-height: 28px; padding: 2px 8px; }
QPushButton:hover { border-color: #9bded2; }
QPushButton#apply { background: #8ce4cb; color: #142720; border-color: #8ce4cb; font-weight: 600; padding-left: 16px; padding-right: 16px; }
QPushButton:disabled, QPushButton#apply:disabled { background: #253343; color: #98a9b8; border-color: #33475a; }
QComboBox QAbstractItemView { background: #223242; selection-background-color: #345d53; }
QToolTip { background: #223242; color: #f1f8ff; }
QScrollBar:vertical { width: 10px; background: #172634; margin: 0; }
QScrollBar::handle:vertical { min-height: 24px; background: #435f70; border-radius: 4px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical { background: transparent; }
"""


class Worker(QObject):
    catalog = Signal(object, object, object, str)
    loaded = Signal(object, object, bool, str)
    applied = Signal(object, object, str)
    previewed = Signal(object, str)
    cleared = Signal(str)

    def __init__(self):
        super().__init__()
        self._backend = None

    @property
    def backend(self):
        if self._backend is None:
            self._backend = Backend(QSettings("Pointrill", "Pointrill"))
        return self._backend

    @Slot()
    def scan(self):
        try:
            devices = self.backend.scan()
            self.catalog.emit(devices, self.backend.selected, self.backend.status(), "")
        except DEVICE_ERRORS as error:
            self.catalog.emit([], None, {}, str(error))

    @Slot(object)
    def load(self, key):
        try:
            fields, volatile = self.backend.select(key)
            self.loaded.emit(key, fields, volatile, "")
        except DEVICE_ERRORS as error:
            self.loaded.emit(key, [], False, str(error))

    @Slot(object, object)
    def apply(self, key, changes):
        try:
            fields = self.backend.apply(key, changes)
            self.applied.emit(key, fields, "")
        except DEVICE_ERRORS as error:
            self.applied.emit(key, [], str(error))

    @Slot(object, object)
    def preview(self, key, value):
        try:
            if key == self.backend.selected:
                if value is None:
                    self.backend.clear_preview()
                else:
                    self.backend.preview_dpi(key, value)
        except DEVICE_ERRORS as error:
            self.previewed.emit(key, str(error))

    @Slot()
    def clear(self):
        try:
            if self._backend is not None:
                self.backend.clear_preview()
            self.cleared.emit("")
        except DEVICE_ERRORS as error:
            self.cleared.emit(str(error))


class DevicePreview(QWidget):
    def __init__(self):
        super().__init__()
        self.receiver = False
        self.photo = QPixmap(str(Path(__file__).with_name("assets") / "basilisk.webp"))
        self.use_photo = False
        self.zone = ""
        self.tint = QColor("#8ce4cb")
        self.setMinimumHeight(170)
        self.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Expanding)
        self.setAccessibleName("Gerätevorschau")

    def paintEvent(self, _event):
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        painter.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
        area = QRectF(self.rect()).adjusted(14, 4, -14, -4)
        if self.use_photo and not self.photo.isNull():
            photo = self.photo.scaled(area.size().toSize(), Qt.AspectRatioMode.KeepAspectRatio,
                                      Qt.TransformationMode.SmoothTransformation)
            x = area.center().x() - photo.width() / 2
            y = area.center().y() - photo.height() / 2
            painter.drawPixmap(int(x), int(y), photo)
            anchors = {"scroll": (.3168, .2406), "logo": (.6723, .5718),
                       "backlight": (.7411, .8368), "thumb": (.218, .5073)}
            if self.zone in anchors:
                px, py = anchors[self.zone]
                self.marker(painter, x + px * photo.width(), y + py * photo.height())
            return
        center = area.center()
        if self.receiver:
            shape = QRectF(center.x() - 37, center.y() - 26, 74, 60)
            painter.setPen(QPen(QColor("#8aafbf"), 2))
            painter.setBrush(QColor("#254255"))
            painter.drawRoundedRect(shape, 13, 13)
            painter.drawRoundedRect(QRectF(center.x() - 16, center.y() - 42, 32, 16), 3, 3)
            self.marker(painter, center.x(), center.y() + 11)
        else:
            shape = QRectF(center.x() - 47, center.y() - 66, 94, 132)
            painter.setPen(QPen(QColor("#8aafbf"), 2))
            painter.setBrush(QColor("#254255"))
            painter.drawRoundedRect(shape, 45, 45)
            painter.drawLine(center.x() - 45, center.y() - 15, center.x() + 45, center.y() - 15)
            painter.drawLine(center.x(), center.y() - 65, center.x(), center.y() - 18)
            painter.setBrush(QColor("#0d1a25"))
            painter.drawRoundedRect(QRectF(center.x() - 6, center.y() - 53, 12, 22), 6, 6)
            if self.zone:
                marker_y = {"scroll": -42, "logo": 24, "backlight": 61}.get(self.zone, 0)
                self.marker(painter, center.x(), center.y() + marker_y)

    def marker(self, painter, x, y):
        painter.setBrush(self.tint)
        painter.setPen(QPen(QColor("#1a282e"), 2))
        painter.drawEllipse(QRectF(x - 5, y - 5, 10, 10))


class Window(QMainWindow):
    scan_requested = Signal()
    load_requested = Signal(object)
    apply_requested = Signal(object, object)
    preview_requested = Signal(object, object)
    clear_requested = Signal()

    def __init__(self):
        super().__init__()
        self.setWindowTitle("Pointrill")
        self.resize(990, 665)
        self.setMinimumSize(QSize(690, 460))
        self.devices = {}
        self.selected = None
        self.editors = {}
        self.catalog_pending = False
        self.busy = False
        self.closing = False
        self.ready_to_close = False
        self.volatile = False
        self.last_preview = None
        self.wheel_note = None
        self.build()
        self.thread = QThread(self)
        self.worker = Worker()
        self.worker.moveToThread(self.thread)
        self.thread.finished.connect(self.worker.deleteLater)
        self.scan_requested.connect(self.worker.scan)
        self.load_requested.connect(self.worker.load)
        self.apply_requested.connect(self.worker.apply)
        self.preview_requested.connect(self.worker.preview)
        self.clear_requested.connect(self.worker.clear)
        self.worker.catalog.connect(self.show_catalog)
        self.worker.loaded.connect(self.show_fields)
        self.worker.applied.connect(self.show_result)
        self.worker.previewed.connect(self.preview_error)
        self.worker.cleared.connect(self.finish_close)
        self.thread.start()
        self.timer = QTimer(self, interval=2500)
        self.timer.timeout.connect(self.scan)
        self.timer.start()
        self.preview_timer = QTimer(self, interval=150, singleShot=True)
        self.preview_timer.timeout.connect(self.send_preview)
        self.scan()

    def build(self):
        canvas = QWidget(objectName="canvas")
        self.setCentralWidget(canvas)
        shell = QHBoxLayout(canvas)
        shell.setContentsMargins(0, 0, 0, 0)
        shell.setSpacing(0)
        sidebar = QWidget(objectName="sidebar")
        sidebar.setFixedWidth(184)
        side = QVBoxLayout(sidebar)
        side.setContentsMargins(15, 20, 15, 15)
        side.addWidget(QLabel("Pointrill", objectName="brand"))
        side.addWidget(QLabel("OpenRazer", objectName="subtitle"))
        side.addSpacing(15)
        side.addWidget(QLabel("Geräte", objectName="eyebrow"))
        self.device_list = QListWidget()
        self.device_list.setWordWrap(True)
        self.device_list.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.device_list.currentItemChanged.connect(self.choose)
        side.addWidget(self.device_list, 1)
        shell.addWidget(sidebar)

        main = QVBoxLayout()
        main.setContentsMargins(21, 19, 21, 17)
        main.setSpacing(12)
        shell.addLayout(main, 1)
        self.heading = QLabel("Maus verbinden", objectName="deviceTitle")
        self.heading.setWordWrap(True)
        main.addWidget(self.heading)
        self.connection = QLabel("OpenRazer wird abgefragt …", objectName="muted")
        self.connection.setWordWrap(True)
        connection_row = QHBoxLayout()
        connection_row.addWidget(self.connection, 1)
        self.reload_button = QPushButton("Neu laden")
        self.reload_button.setEnabled(False)
        self.reload_button.clicked.connect(self.reload)
        connection_row.addWidget(self.reload_button)
        main.addLayout(connection_row)
        self.live = QLabel(objectName="eyebrow")
        self.live.setWordWrap(True)
        main.addWidget(self.live)
        middle = QHBoxLayout()
        middle.setSpacing(13)
        main.addLayout(middle, 1)
        self.tabs = QTabWidget()
        self.tabs.currentChanged.connect(self.update_preview)
        middle.addWidget(self.tabs, 3)

        self.preview_panel = QFrame(objectName="preview")
        self.preview_panel.setMinimumWidth(155)
        self.preview_panel.setMaximumWidth(272)
        preview = QVBoxLayout(self.preview_panel)
        preview.setContentsMargins(12, 10, 12, 10)
        preview.addWidget(QLabel("Gerätevorschau", objectName="fieldTitle"))
        self.art = DevicePreview()
        preview.addWidget(self.art, 1)
        self.art_caption = QLabel(objectName="muted", wordWrap=True)
        preview.addWidget(self.art_caption)
        middle.addWidget(self.preview_panel, 2)
        self.preview_panel.hide()

        bottom = QHBoxLayout()
        self.message = QLabel("Warte auf OpenRazer …", objectName="muted", wordWrap=True)
        bottom.addWidget(self.message, 1)
        self.discard_button = QPushButton("Verwerfen")
        self.apply_button = QPushButton("Übernehmen", objectName="apply")
        self.discard_button.setEnabled(False)
        self.apply_button.setEnabled(False)
        self.discard_button.clicked.connect(self.discard)
        self.apply_button.clicked.connect(self.apply)
        bottom.addWidget(self.discard_button)
        bottom.addWidget(self.apply_button)
        main.addLayout(bottom)

    @Slot()
    def scan(self):
        if not self.catalog_pending and not self.busy and not self.closing:
            self.catalog_pending = True
            self.scan_requested.emit()

    @Slot(object, object, object, str)
    def show_catalog(self, devices, backend_key, status, error):
        self.catalog_pending = False
        if self.busy or self.closing:
            return
        catalog = {device.key: device for device in devices}
        if catalog != self.devices:
            self.devices = catalog
            self.device_list.blockSignals(True)
            self.device_list.clear()
            for info in devices:
                item = QListWidgetItem(info.name)
                item.setData(Qt.ItemDataRole.UserRole, info.key)
                self.device_list.addItem(item)
            self.highlight(self.selected)
            self.device_list.blockSignals(False)
        if not devices:
            self.selected = None
            self.last_preview = None
            self.preview_timer.stop()
            self.heading.setText("Maus verbinden")
            self.connection.setText("OpenRazer ist gerade nicht erreichbar." if error else "Warte auf eine unterstützte Maus oder einen Empfänger.")
            self.live.clear()
            self.message.setText("Geräte werden automatisch erkannt.")
            self.set_fields([])
            self.preview_panel.hide()
            self.reload_button.setEnabled(False)
            return
        if self.selected not in catalog:
            self.select(devices[0].key)
        elif backend_key != self.selected and not self.busy:
            self.select(self.selected)
        elif backend_key == self.selected:
            values = []
            if "battery_level" in status:
                charging = " · lädt" if status.get("is_charging") else ""
                values.append(f"Akku {status['battery_level']} %{charging}")
            if "dpi" in status:
                x, y = status["dpi"]
                values.append(f"{x} DPI" if not y or x == y else f"{x} × {y} DPI")
            if "poll_rate" in status:
                values.append(f"{status['poll_rate']} Hz")
            self.live.setText("   |   ".join(values))

    def highlight(self, key):
        was_blocked = self.device_list.blockSignals(True)
        for index in range(self.device_list.count()):
            item = self.device_list.item(index)
            if tuple(item.data(Qt.ItemDataRole.UserRole)) == key:
                self.device_list.setCurrentItem(item)
                break
        self.device_list.blockSignals(was_blocked)

    def confirm_discard(self):
        return not self.changes() or QMessageBox.question(
            self, "Änderungen verwerfen?", "Sollen die Änderungen im Entwurf verworfen werden?",
            QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel,
            QMessageBox.StandardButton.Cancel,
        ) == QMessageBox.StandardButton.Discard

    def choose(self, item, previous):
        if item is None:
            return
        key = tuple(item.data(Qt.ItemDataRole.UserRole))
        if key == self.selected:
            return
        if self.busy or not self.confirm_discard():
            self.highlight(self.selected)
            return
        self.select(key)

    def select(self, key):
        self.preview_timer.stop()
        self.last_preview = None
        self.volatile = False
        self.selected = key
        self.highlight(key)
        info = self.devices[key]
        self.heading.setText(info.name)
        self.connection.setText(f"{info.connection} · {info.ids[0]:04X}:{info.ids[1]:04X}")
        self.live.clear()
        self.art.receiver = any(word in info.name.lower() for word in ("dongle", "receiver", "dock"))
        self.art.use_photo = info.ids in BASILISK
        self.preview_panel.show()
        self.set_busy(True, "Lese Geräteeinstellungen …")
        self.load_requested.emit(key)

    @Slot(object, object, bool, str)
    def show_fields(self, key, fields, volatile, error):
        if key != self.selected or self.closing:
            return
        self.volatile = volatile
        self.set_fields(fields)
        self.set_busy(False, error or ("Gerät bereit." if fields else "Für dieses Gerät sind gerade keine Werte bearbeitbar."))
        self.scan()

    def set_fields(self, fields):
        tab_name = self.tabs.tabText(self.tabs.currentIndex())
        self.editors.clear()
        self.wheel_note = None
        self.tabs.blockSignals(True)
        while self.tabs.count():
            widget = self.tabs.widget(0)
            self.tabs.removeTab(0)
            widget.deleteLater()
        buckets = {}
        for field in fields:
            if field.page not in buckets:
                pane = QWidget(objectName="settingsPane")
                layout = QVBoxLayout(pane)
                layout.setContentsMargins(3, 8, 7, 2)
                region = QScrollArea(widgetResizable=True)
                region.setWidget(pane)
                region.viewport().setObjectName("settingsViewport")
                self.tabs.addTab(region, field.page)
                buckets[field.page] = layout
            editor = Editor(field)
            editor.changed.connect(self.edit_changed)
            buckets[field.page].addWidget(editor)
            self.editors[field.key] = editor
        if "cycle" in self.editors:
            self.wheel_note = QLabel("Smart Reel und Scrollbeschleunigung sind bei einer Auswahl mit Freilauf verfügbar.",
                                    objectName="muted", wordWrap=True)
            buckets["Scrollrad"].addWidget(self.wheel_note)
        for layout in buckets.values():
            layout.addStretch(1)
        for index in range(self.tabs.count()):
            if self.tabs.tabText(index) == tab_name:
                self.tabs.setCurrentIndex(index)
        self.tabs.blockSignals(False)
        self.update_wheel()
        self.update_preview()
        self.update_actions()

    def update_wheel(self):
        compatible = self.free_spin_options_available()
        for key in (*ASSISTS, "sensitivity"):
            if key in self.editors:
                self.editors[key].setVisible(compatible)
        if self.wheel_note is not None:
            self.wheel_note.setVisible(not compatible)

    def free_spin_options_available(self):
        selector = self.editors.get("cycle")
        return selector is None or bool((selector.value() or 0) & 2)

    def changes(self):
        ignored = () if self.free_spin_options_available() else (*ASSISTS, "sensitivity")
        changes = {key: editor.value() for key, editor in self.editors.items()
                   if key not in ignored and editor.value() != editor.initial}
        return changes

    def update_actions(self):
        enable = bool(self.changes()) and not self.busy and not self.closing
        self.apply_button.setEnabled(enable)
        self.discard_button.setEnabled(enable)

    def edit_changed(self):
        self.update_wheel()
        self.update_preview()
        self.update_actions()
        if self.busy or self.closing:
            return
        changes = self.changes()
        self.message.setText("Änderungen ausstehend." if changes else "Gerät bereit.")
        if self.volatile:
            self.preview_timer.start()

    def update_preview(self):
        tab = self.tabs.tabText(self.tabs.currentIndex())
        self.art.zone = "thumb" if tab == "Scrollrad" and self.art.use_photo else ""
        self.art.tint = QColor("#8ce4cb")
        summary = ""
        if tab == "Beleuchtung":
            lights = [editor for key, editor in self.editors.items() if key.startswith("light:")]
            editor = next((item for item in lights if item.value() != item.initial), None)
            if editor is not None:
                name, color, _ = editor.value()
                self.art.zone = editor.field.key.split(":")[-1]
                summary = f"Vorschau · {editor.field.label}"
                if color:
                    self.art.tint = QColor(*color)
                elif name == "none":
                    self.art.tint = QColor("#6d8093")
        elif tab == "Scrollrad" and self.art.use_photo:
            summary = "Beispielbelegung · vordere Daumentaste"
        elif tab == "Sensor":
            summary = "DPI kann direkt vorgeführt werden." if self.volatile else "Sensorwerte als Entwurf."
        self.art_caption.setText(summary)
        self.art_caption.setVisible(bool(summary))
        self.art.update()

    def send_preview(self):
        if self.busy or self.closing or not self.volatile:
            return
        changes = self.changes()
        value = changes.get("dpi")
        if "stages" in changes:
            stage, points = changes["stages"]
            value = points[stage - 1]
        if value != self.last_preview:
            self.last_preview = value
            self.preview_requested.emit(self.selected, value)

    @Slot(object, str)
    def preview_error(self, key, error):
        if key == self.selected:
            self.message.setText(error)

    def set_busy(self, busy, message):
        self.busy = busy
        self.tabs.setEnabled(not busy)
        self.device_list.setEnabled(not busy)
        self.reload_button.setEnabled(not busy and self.selected is not None)
        self.message.setText(message)
        self.update_actions()

    def apply(self):
        if self.busy:
            return
        changes = self.changes()
        if changes:
            self.preview_timer.stop()
            self.last_preview = None
            self.set_busy(True, "Übernehme Einstellungen …")
            self.apply_requested.emit(self.selected, changes)

    @Slot(object, object, str)
    def show_result(self, key, fields, error):
        if key != self.selected or self.closing:
            return
        if not error:
            self.set_fields(fields)
        self.set_busy(False, error or "Einstellungen übernommen.")
        self.scan()

    def discard(self):
        if not self.busy and self.selected in self.devices:
            self.select(self.selected)

    def reload(self):
        if self.confirm_discard():
            self.discard()

    def closeEvent(self, event):
        if self.ready_to_close:
            event.accept()
            return
        event.ignore()
        if self.busy or self.closing or not self.confirm_discard():
            return
        self.closing = True
        self.timer.stop()
        self.preview_timer.stop()
        self.set_busy(True, "Beende die Gerätevorschau …")
        self.clear_requested.emit()

    @Slot(str)
    def finish_close(self, error):
        if error:
            self.closing = False
            self.set_busy(False, error)
            self.timer.start()
            return
        self.thread.quit()
        self.thread.wait()
        self.ready_to_close = True
        self.close()


def main():
    logging.basicConfig(level=logging.INFO)
    app = QApplication(sys.argv)
    app.setApplicationName("Pointrill")
    app.setStyle("Fusion")
    app.setStyleSheet(STYLE)
    window = Window()
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
