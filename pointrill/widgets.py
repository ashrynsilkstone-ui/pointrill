"""Editors generated from OpenRazer capabilities."""

from PySide6.QtCore import Signal, Qt
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QCheckBox, QColorDialog, QComboBox, QGridLayout, QHBoxLayout, QLabel,
    QPushButton, QSlider, QSpinBox, QVBoxLayout, QWidget,
)

from .backend import COLOR_EFFECTS, DIRECTION_EFFECTS


def combo_box(choices, value):
    control = QComboBox()
    if value not in dict(choices):
        label = "Auswählen …" if value is None else f"Aktuell ({value})"
        control.addItem(label, value)
    for key, text in choices:
        control.addItem(text, key)
    control.setCurrentIndex(control.findData(value))
    return control


def spin_box(minimum, maximum, value, suffix=""):
    control = QSpinBox()
    control.setRange(min(minimum, value), max(maximum, value))
    control.setValue(value)
    control.setSuffix(suffix)
    return control


class Editor(QWidget):
    changed = Signal()

    def __init__(self, field):
        super().__init__()
        self.field = field
        self.setObjectName("editor")
        self.setAttribute(Qt.WidgetAttribute.WA_StyledBackground, True)
        self.layout_box = QVBoxLayout(self)
        self.layout_box.setContentsMargins(13, 10, 13, 10)
        self.layout_box.setSpacing(8)
        self.title = QLabel(field.label)
        self.title.setObjectName("fieldTitle")
        self.layout_box.addWidget(self.title)
        self.make_control()
        self.initial = self.value()

    def make_control(self):
        field = self.field
        kind = field.kind
        value = field.value
        if kind == "choice":
            self.control = combo_box(field.choices, value)
            self.control.currentIndexChanged.connect(self.changed)
            self.title.setBuddy(self.control)
            self.layout_box.addWidget(self.control)
        elif kind == "number":
            suffix = " s" if field.key == "sleep" else " %"
            self.control = spin_box(field.minimum, field.maximum, int(value), suffix)
            self.control.valueChanged.connect(self.changed)
            self.title.setBuddy(self.control)
            self.layout_box.addWidget(self.control)
        elif kind == "bool":
            self.title.hide()
            self.control = QCheckBox(field.label)
            self.control.setChecked(value)
            self.control.toggled.connect(self.changed)
            self.layout_box.addWidget(self.control)
        elif kind == "brightness":
            self.control = QSlider(Qt.Orientation.Horizontal)
            self.control.setRange(field.minimum, field.maximum)
            self.control.setValue(round(value))
            self.readout = QLabel(f"{self.control.value()} %")
            self.title.setBuddy(self.control)
            self.control.valueChanged.connect(self.set_brightness)
            row = QHBoxLayout()
            row.addWidget(self.control)
            row.addWidget(self.readout)
            self.layout_box.addLayout(row)
        elif kind == "stages":
            self.make_stages()
        elif kind == "dpi":
            self.make_dpi()
        elif kind == "indicators":
            self.controls = []
            for index, position in enumerate(value, 1):
                label = QLabel(f"LED {index}")
                choice = combo_box(field.choices, position)
                label.setBuddy(choice)
                choice.currentIndexChanged.connect(self.changed)
                row = QHBoxLayout()
                row.addWidget(label)
                row.addWidget(choice, 1)
                self.layout_box.addLayout(row)
                self.controls.append(choice)
        elif kind == "light":
            self.make_light()

    def set_brightness(self, value):
        self.readout.setText(f"{value} %")
        self.changed.emit()

    def dpi_axes(self, parent_layout, value, axes=True):
        controls = []
        for label, axis in zip(("X", "Y") if axes else ("DPI",), value):
            control = spin_box(self.field.minimum, self.field.maximum, axis)
            control.setAccessibleName(f"DPI {label}")
            control.valueChanged.connect(self.changed)
            parent_layout.addWidget(control)
            controls.append(control)
        return tuple(controls)

    def make_dpi(self):
        value = self.field.value
        if self.field.choices:
            self.control = combo_box(self.field.choices, value[0])
            self.control.currentIndexChanged.connect(self.changed)
            self.title.setBuddy(self.control)
            self.layout_box.addWidget(self.control)
        else:
            controls = QHBoxLayout()
            self.axes = self.dpi_axes(controls, value, value[1] > 0)
            self.layout_box.addLayout(controls)

    def make_stages(self):
        active, stages = self.field.value
        self.count = spin_box(1, 5, len(stages))
        self.count.valueChanged.connect(self.update_stages)
        self.active = QComboBox()
        self.active.currentIndexChanged.connect(self.changed)
        properties = QGridLayout()
        for column, (name, control) in enumerate((("Anzahl", self.count), ("Aktive Stufe", self.active))):
            label = QLabel(name)
            label.setBuddy(control)
            properties.addWidget(label, 0, column)
            properties.addWidget(control, 1, column)
        self.layout_box.addLayout(properties)
        grid = QGridLayout()
        grid.addWidget(QLabel("Stufe"), 0, 0)
        grid.addWidget(QLabel("DPI X"), 0, 1)
        grid.addWidget(QLabel("DPI Y"), 0, 2)
        self.stages = []
        for index in range(5):
            x, y = stages[min(index, len(stages) - 1)]
            label = QLabel(str(index + 1))
            controls = spin_box(self.field.minimum, self.field.maximum, x), spin_box(self.field.minimum, self.field.maximum, y)
            grid.addWidget(label, index + 1, 0)
            for column, (axis, control) in enumerate(zip(("X", "Y"), controls), 1):
                control.setAccessibleName(f"Stufe {index + 1}, DPI {axis}")
                control.valueChanged.connect(self.changed)
                grid.addWidget(control, index + 1, column)
            self.stages.append((label, *controls))
        self.layout_box.addLayout(grid)
        self.update_stages(active=active)

    def update_stages(self, _=0, *, active=None):
        if active is None:
            active = self.active.currentData() or 1
        self.active.blockSignals(True)
        self.active.clear()
        for index, row in enumerate(self.stages, 1):
            for item in row:
                item.setVisible(index <= self.count.value())
            if index <= self.count.value():
                self.active.addItem(str(index), index)
        self.active.setCurrentIndex(max(0, min(active, self.count.value()) - 1))
        self.active.blockSignals(False)
        self.changed.emit()

    def make_light(self):
        effect, color, direction = self.field.value
        self.control = combo_box(self.field.choices, effect)
        self.control.currentIndexChanged.connect(self.show_effect_controls)
        self.title.setBuddy(self.control)
        self.layout_box.addWidget(self.control)
        self.color = tuple(color or (109, 228, 194))
        self.color_picker = QPushButton()
        self.color_picker.clicked.connect(self.choose_color)
        self.layout_box.addWidget(self.color_picker)
        self.direction = combo_box(((1, "Richtung 1"), (2, "Richtung 2")), direction or 1)
        self.direction.setAccessibleName("Effektrichtung")
        self.direction.currentIndexChanged.connect(self.changed)
        self.layout_box.addWidget(self.direction)
        self.show_effect_controls()

    def show_effect_controls(self):
        effect = self.control.currentData()
        self.color_picker.setVisible(effect in COLOR_EFFECTS)
        self.direction.setVisible(effect in DIRECTION_EFFECTS)
        self.color_picker.setText(f"Farbe ändern · {QColor(*self.color).name().upper()}")
        self.changed.emit()

    def choose_color(self):
        result = QColorDialog.getColor(QColor(*self.color), self, "Beleuchtungsfarbe")
        if result.isValid():
            self.color = result.red(), result.green(), result.blue()
            self.show_effect_controls()

    def value(self):
        kind = self.field.kind
        if kind == "choice":
            return self.control.currentData()
        if kind in ("number", "brightness"):
            return self.control.value()
        if kind == "bool":
            return self.control.isChecked()
        if kind == "stages":
            return self.active.currentData(), tuple((x.value(), y.value()) for _, x, y in self.stages[:self.count.value()])
        if kind == "dpi":
            if self.field.choices:
                return self.control.currentData(), 0
            x = self.axes[0].value()
            return x, self.axes[1].value() if len(self.axes) == 2 else 0
        if kind == "indicators":
            return tuple(control.currentData() for control in self.controls)
        effect = self.control.currentData()
        return (effect, self.color if effect in COLOR_EFFECTS else None,
                self.direction.currentData() if effect in DIRECTION_EFFECTS else None)
