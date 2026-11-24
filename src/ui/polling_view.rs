use std::collections::VecDeque;

use eframe::egui::{
    self, Color32 as Color, FontId, Pos2, Rect, RichText as Text, Stroke, Ui, vec2,
};

use super::{BASE, GREEN, LINE, MUTED, PANEL, WARN, button, emphasis, tile};
use crate::model::Device;
use crate::polling::{HISTORY, PollingTest, Sample};

const CPU_COLOR: Color = Color::from_rgb(103, 203, 250);

#[derive(Default)]
pub(super) struct PollingView {
    test: Option<PollingTest>,
    error: Option<String>,
}

impl PollingView {
    pub(super) fn refresh(&mut self) {
        if let Some(test) = &mut self.test {
            test.refresh();
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(test) = &mut self.test {
            test.stop();
        }
    }

    pub(super) fn show(&mut self, ui: &mut Ui, device: &Device, configured: Option<i32>) {
        tile(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let running = self.test.as_ref().is_some_and(PollingTest::running);
            ui.horizontal_wrapped(|ui| {
                ui.label(emphasis("Polling-Test", 20.0));
                if running {
                    if button(ui, "Messung stoppen", false, (126.0, 32.0)).clicked() {
                        self.stop();
                    }
                } else if button(ui, "Messung starten", true, (126.0, 32.0)).clicked() {
                    self.test = None;
                    self.error = None;
                    let ctx = ui.ctx().clone();
                    match PollingTest::start(device, move || ctx.request_repaint()) {
                        Ok(test) => self.test = Some(test),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            });
            if running {
                ui.label(
                    Text::new("Maus kurz stillhalten, dann schnell hin und her bewegen.")
                        .color(MUTED),
                );
            } else if self.test.is_none() && self.error.is_none() {
                ui.label(
                    Text::new(
                        "Miss die Bewegungsberichte der ausgewählten Maus und die CPU-Gesamtlast.",
                    )
                    .color(MUTED),
                );
            }
            if let Some(error) = &self.error {
                ui.label(Text::new(error).color(WARN));
            }
            if let Some(test) = &self.test {
                show_samples(ui, test, configured);
            }
        });
    }
}

fn show_samples(ui: &mut Ui, test: &PollingTest, configured: Option<i32>) {
    let samples = &test.samples;
    let Some(latest) = samples.back() else {
        if let Some(message) = test.message {
            ui.label(Text::new(message).color(WARN));
        } else {
            ui.label(Text::new("Warte auf erste Messwerte …").color(MUTED));
        }
        return;
    };
    let configured = configured.filter(|hz| *hz > 0).map(f64::from);
    let peak = samples
        .iter()
        .filter_map(|sample| sample.hz)
        .reduce(f64::max);
    let hz_top = (peak.unwrap_or(0.0).max(configured.unwrap_or(0.0)) / 250.0)
        .ceil()
        .max(1.0)
        * 250.0;
    let (idle_cpu_sum, idle_count) = samples
        .iter()
        .filter(|sample| sample.hz == Some(0.0))
        .filter_map(|sample| sample.cpu)
        .fold((0.0, 0), |(sum, count), cpu| (sum + cpu, count + 1));
    let idle_cpu = (idle_count >= 4).then(|| idle_cpu_sum / f64::from(idle_count));
    let cpu_peak = samples
        .iter()
        .filter_map(|sample| sample.cpu)
        .reduce(f64::max);
    let cpu_top = (cpu_peak.unwrap_or(0.0) / 10.0).ceil().clamp(1.0, 10.0) * 10.0;

    let motion = Chart {
        title: "Bewegungsrate",
        current: latest
            .hz
            .map_or_else(|| "–".into(), |hz| format!("{hz:.0} Hz")),
        details: match (configured, peak.filter(|hz| *hz > 0.0)) {
            (Some(configured), Some(peak)) => {
                format!("Eingestellt {configured:.0} Hz · Spitze {peak:.0} Hz")
            }
            (Some(configured), None) => format!("Eingestellt {configured:.0} Hz"),
            (None, Some(peak)) => format!("Spitze {peak:.0} Hz"),
            (None, None) => String::new(),
        },
        color: GREEN,
        ceiling: hz_top,
        unit: "Hz",
        value: |sample| sample.hz,
        reference: configured,
    };
    let cpu = Chart {
        title: "CPU-Gesamtlast",
        current: latest
            .cpu
            .map_or_else(|| "–".into(), |cpu| format!("{cpu:.1} %")),
        details: idle_cpu.map_or_else(String::new, |cpu| format!("Ohne Mausbewegung {cpu:.1} %")),
        color: CPU_COLOR,
        ceiling: cpu_top,
        unit: "%",
        value: |sample| sample.cpu,
        reference: idle_cpu,
    };
    ui.add_space(5.0);
    if ui.available_width() >= 665.0 {
        ui.columns(2, |columns| {
            motion.show(&mut columns[0], samples, test.running());
            cpu.show(&mut columns[1], samples, test.running());
        });
    } else {
        motion.show(ui, samples, test.running());
        ui.add_space(8.0);
        cpu.show(ui, samples, test.running());
    }
    if latest.hz.is_none() {
        ui.label(Text::new("Messlücke durch verlorene Eingabeereignisse.").color(WARN));
    }
    if let Some(message) = test.message {
        ui.label(Text::new(message).color(WARN));
    }
    ui.label(
        Text::new(
            "250-ms-Messfenster · Bewegungsberichte am Linux-Eingabegerät · CPU über alle Kerne",
        )
        .color(MUTED)
        .size(12.0),
    );
}

struct Chart {
    title: &'static str,
    current: String,
    details: String,
    color: Color,
    ceiling: f64,
    unit: &'static str,
    value: fn(&Sample) -> Option<f64>,
    reference: Option<f64>,
}

impl Chart {
    fn show(&self, ui: &mut Ui, samples: &VecDeque<Sample>, running: bool) {
        tile(BASE).inner_margin(9).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(Text::new(self.title).color(MUTED).size(13.0));
            ui.horizontal_wrapped(|ui| {
                ui.label(emphasis(&self.current, 25.0).color(self.color));
                if !self.details.is_empty() {
                    ui.label(Text::new(&self.details).color(MUTED).size(12.0));
                }
            });
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 115.0), egui::Sense::hover());
            self.paint(ui.painter(), rect, samples);
            ui.horizontal(|ui| {
                ui.label(Text::new("−15 s").color(MUTED).size(11.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        Text::new(if running { "Jetzt" } else { "Ende" })
                            .color(MUTED)
                            .size(11.0),
                    );
                });
            });
        });
    }

    fn paint(&self, painter: &egui::Painter, rect: Rect, samples: &VecDeque<Sample>) {
        let plot = Rect::from_min_max(
            rect.left_top() + vec2(56.0, 8.0),
            rect.right_bottom() - vec2(6.0, 7.0),
        );
        let y = |value: f64| {
            plot.bottom() - ((value / self.ceiling) as f32).clamp(0.0, 1.0) * plot.height()
        };
        for value in [self.ceiling, self.ceiling / 2.0, 0.0] {
            let y = y(value);
            painter.hline(plot.x_range(), y, Stroke::new(1.0, LINE));
            painter.text(
                Pos2::new(plot.left() - 6.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{value:.0} {}", self.unit),
                FontId::proportional(10.0),
                MUTED,
            );
        }
        if let Some(reference) = self.reference {
            let y = y(reference);
            painter.extend(
                (0..)
                    .map(|index| plot.left() + index as f32 * 9.0)
                    .take_while(|x| *x < plot.right())
                    .map(|x| {
                        egui::Shape::line_segment(
                            [Pos2::new(x, y), Pos2::new((x + 4.0).min(plot.right()), y)],
                            Stroke::new(1.0, MUTED),
                        )
                    }),
            );
        }
        let Some(latest) = samples.back() else {
            return;
        };
        let mut previous: Option<(f64, Pos2)> = None;
        for sample in samples {
            let Some(value) = (self.value)(sample) else {
                previous = None;
                continue;
            };
            let x = plot.right()
                - (((latest.elapsed - sample.elapsed) / HISTORY) as f32).clamp(0.0, 1.0)
                    * plot.width();
            let current = Pos2::new(x, y(value));
            if let Some((time, from)) = previous
                && sample.elapsed - time < 0.6
            {
                painter.line_segment([from, current], Stroke::new(1.8, self.color));
            }
            previous = Some((sample.elapsed, current));
        }
        if let Some((time, point)) = previous
            && time == latest.elapsed
        {
            painter.circle_filled(point, 2.6, self.color);
        }
    }
}
