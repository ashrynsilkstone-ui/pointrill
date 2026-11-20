use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32 as Color, FontId, Pos2, Rect, RichText as Text, Stroke, Ui, Vec2, vec2,
};

use crate::model::{
    Control, Device, Dpi, Field, Key, Page, Snapshot, Status, Value, draft_changes, has_free_spin,
};
use crate::photos::Loader;
use crate::worker::{Reply, Request, Worker};

const BASE: Color = Color::from_rgb(12, 15, 24);
const RAIL: Color = Color::from_rgb(16, 19, 29);
const PANEL: Color = Color::from_rgb(21, 25, 38);
const FIELD: Color = Color::from_rgb(26, 31, 47);
const LINE: Color = Color::from_rgb(49, 57, 79);
const TEXT: Color = Color::from_rgb(242, 244, 252);
const MUTED: Color = Color::from_rgb(160, 171, 193);
const VIOLET: Color = Color::from_rgb(182, 160, 255);
const VIOLET_FILL: Color = Color::from_rgb(54, 43, 91);
const ACID: Color = Color::from_rgb(208, 250, 116);
const WARN: Color = Color::from_rgb(255, 164, 151);

pub struct App {
    worker: Worker,
    devices: Vec<Device>,
    snapshot: Option<Snapshot>,
    draft: Vec<Value>,
    status: Status,
    page: Page,
    message: String,
    problem: bool,
    busy: bool,
    scanning: bool,
    next_scan: Instant,
    preview_sent: Option<Dpi>,
    preview_deadline: Option<Instant>,
    confirm: Option<Pending>,
    closing: bool,
    ready_to_close: bool,
    photos: Loader,
    photo_url: Option<String>,
    picture: ProductPicture,
    capture: Option<(PathBuf, Instant)>,
}

enum ProductPicture {
    Missing,
    Loading,
    Ready(egui::TextureHandle),
    Failed(String),
}

enum Pending {
    Switch(Device),
    Reload,
    Close,
}

impl App {
    pub fn new(context: &egui::Context, capture: Option<PathBuf>) -> Self {
        theme(context);
        let repaint = context.clone();
        let worker = Worker::start(move || repaint.request_repaint());
        let repaint = context.clone();
        let photos = Loader::start(move || repaint.request_repaint());
        Self {
            worker,
            devices: vec![],
            snapshot: None,
            draft: vec![],
            status: Status::default(),
            page: Page::Sensor,
            message: "Suche nach OpenRazer-Geräten …".into(),
            problem: false,
            busy: false,
            scanning: false,
            next_scan: Instant::now(),
            preview_sent: None,
            preview_deadline: None,
            confirm: None,
            closing: false,
            ready_to_close: false,
            photos,
            photo_url: None,
            picture: ProductPicture::Missing,
            capture: capture.map(|path| (path, Instant::now() + Duration::from_millis(1400))),
        }
    }

    fn changes(&self) -> Vec<(Key, Value)> {
        self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
            draft_changes(&snapshot.fields, &self.draft)
        })
    }

    fn notify(&mut self, message: impl Into<String>, problem: bool) {
        self.message = message.into();
        self.problem = problem;
    }

    fn load(&mut self, device: Device) {
        self.busy = true;
        self.preview_sent = None;
        self.preview_deadline = None;
        self.notify("Geräteeinstellungen werden gelesen …", false);
        let _ = self.worker.send.send(Request::Open(device));
    }

    fn accept_snapshot(&mut self, snapshot: Snapshot) {
        let same = self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.device == snapshot.device);
        self.draft = snapshot
            .fields
            .iter()
            .map(|field| field.value.clone())
            .collect();
        if !same {
            self.status = Status::default();
        }
        self.set_photo(snapshot.product_image.clone());
        self.snapshot = Some(snapshot);
        if !self.has_page(self.page) {
            self.page = Page::ALL
                .into_iter()
                .find(|page| self.has_page(*page))
                .unwrap_or(Page::Sensor);
        }
        self.preview_sent = None;
        self.preview_deadline = None;
        self.next_scan = Instant::now();
    }

    fn set_photo(&mut self, url: Option<String>) {
        if self.photo_url != url {
            self.photo_url = url;
            self.request_photo();
        }
    }

    fn request_photo(&mut self) {
        self.picture = if let Some(url) = &self.photo_url {
            if self.photos.requests.send(url.clone()).is_ok() {
                ProductPicture::Loading
            } else {
                ProductPicture::Failed("Der Bildabruf ist nicht erreichbar.".into())
            }
        } else {
            ProductPicture::Missing
        };
    }

    fn read_photos(&mut self, context: &egui::Context) {
        while let Ok((url, result)) = self.photos.replies.try_recv() {
            if self.photo_url.as_ref() != Some(&url) {
                continue;
            }
            self.picture = match result {
                Ok(photo) => ProductPicture::Ready(context.load_texture(
                    url,
                    photo,
                    egui::TextureOptions::LINEAR,
                )),
                Err(error) => ProductPicture::Failed(format!("{error:#}")),
            };
        }
    }

    fn read_replies(&mut self, context: &egui::Context) {
        while let Ok(reply) = self.worker.replies.try_recv() {
            match reply {
                Reply::Scan(result) => {
                    self.scanning = false;
                    if self.busy || self.closing {
                        continue;
                    }
                    match result {
                        Ok((devices, selected, status)) => {
                            self.devices = devices;
                            self.status = status;
                            let current = self.snapshot.as_ref().map(|view| &view.device);
                            if self.devices.is_empty() {
                                self.snapshot = None;
                                self.set_photo(None);
                                self.draft.clear();
                                self.notify("Warte auf eine Maus oder einen Empfänger.", false);
                            } else if current != selected.as_ref() || current.is_none() {
                                let target = current
                                    .filter(|device| self.devices.contains(device))
                                    .cloned()
                                    .unwrap_or_else(|| self.devices[0].clone());
                                self.load(target);
                            }
                        }
                        Err(error) => {
                            self.devices.clear();
                            self.status = Status::default();
                            self.notify(format!("{error:#}"), true);
                        }
                    }
                }
                Reply::Open(result) => {
                    self.busy = false;
                    match result {
                        Ok(snapshot) => {
                            self.accept_snapshot(snapshot);
                            self.notify("Gerät bereit", false);
                        }
                        Err(error) => self.notify(format!("{error:#}"), true),
                    }
                }
                Reply::Apply(result) => {
                    self.busy = false;
                    match result {
                        Ok(snapshot) => {
                            self.accept_snapshot(snapshot);
                            self.notify("Einstellungen übernommen", false);
                        }
                        Err(error) => self.notify(format!("{error:#}"), true),
                    }
                }
                Reply::Preview(Err(error)) => {
                    self.notify(format!("DPI-Vorschau · {error:#}"), true)
                }
                Reply::Preview(Ok(())) => self.next_scan = Instant::now(),
                Reply::Close(result) => {
                    self.closing = false;
                    match result {
                        Ok(()) => {
                            self.ready_to_close = true;
                            self.worker.join();
                            context.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(error) => self.notify(format!("{error:#}"), true),
                    }
                }
            }
        }
    }

    fn close(&mut self) {
        if !self.closing {
            self.closing = true;
            self.preview_deadline = None;
            self.notify("Beende die DPI-Vorschau …", false);
            let _ = self.worker.send.send(Request::Close);
        }
    }

    fn ask(&mut self, pending: Pending) {
        if self.changes().is_empty() || matches!(pending, Pending::Reload) {
            self.execute(pending);
        } else {
            self.confirm = Some(pending);
        }
    }

    fn execute(&mut self, pending: Pending) {
        match pending {
            Pending::Switch(device) => self.load(device),
            Pending::Reload => {
                if let Some(device) = self.snapshot.as_ref().map(|item| item.device.clone()) {
                    self.load(device);
                }
            }
            Pending::Close => self.close(),
        }
    }

    fn apply(&mut self) {
        let changes = self.changes();
        if changes.is_empty() {
            return;
        }
        self.preview_deadline = None;
        self.preview_sent = None;
        self.busy = true;
        self.notify("Übernehme und prüfe die Einstellungen …", false);
        let _ = self.worker.send.send(Request::Apply(changes));
    }

    fn preview(&mut self) {
        self.preview_deadline = None;
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if !snapshot.volatile_dpi {
            return;
        }
        let next = self
            .changes()
            .iter()
            .find_map(|(_, value)| value.current_dpi());
        if next != self.preview_sent {
            self.preview_sent = next;
            let _ = self.worker.send.send(Request::Preview(next));
        }
    }

    fn has_page(&self, page: Page) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.fields.iter().any(|field| field.page == page))
            || (page == Page::Power && self.status.battery.is_some())
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (mark, _) = ui.allocate_exact_size(vec2(43.0, 40.0), egui::Sense::hover());
            logo(ui.painter(), mark);
            ui.vertical(|ui| {
                ui.label(Text::new("POINTRILL").size(23.0).strong().color(TEXT));
                ui.label(Text::new("OpenRazer").size(11.0).monospace().color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(snapshot) = &self.snapshot {
                    let device = &snapshot.device;
                    ui.label(
                        Text::new(format!("{:04X}:{:04X}", device.vid, device.pid))
                            .monospace()
                            .color(MUTED)
                            .size(12.0),
                    );
                    ui.add_space(9.0);
                    tag(ui, device.connection(), VIOLET);
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.label(Text::new("GERÄTE").monospace().size(11.0).color(MUTED));
        ui.add_space(6.0);
        if self.devices.is_empty() {
            ui.label(Text::new("Gerät verbinden").color(MUTED));
        }
        let mut target = None;
        for device in &self.devices {
            let current = self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.device == *device);
            let name = device.name.trim_start_matches("Razer ");
            let response = button(ui, name, current, (ui.available_width(), 44.0));
            if response.clicked() && !current {
                target = Some(device.clone());
            }
        }
        if let Some(target) = target {
            self.ask(Pending::Switch(target));
        }
        if self.snapshot.is_some() {
            ui.add_space(24.0);
            ui.label(
                Text::new("EINSTELLUNGEN")
                    .monospace()
                    .size(11.0)
                    .color(MUTED),
            );
            ui.add_space(6.0);
            for page in Page::ALL {
                if self.has_page(page)
                    && button(
                        ui,
                        page.title(),
                        self.page == page,
                        (ui.available_width(), 40.0),
                    )
                    .clicked()
                {
                    self.page = page;
                }
            }
        }
    }

    fn main(&mut self, ui: &mut Ui) {
        let Some(snapshot) = &self.snapshot else {
            ui.add_space(66.0);
            ui.vertical_centered(|ui| {
                ui.label(Text::new("Maus verbinden").size(32.0).strong());
                ui.label(
                    Text::new("Unterstützte Mäuse und Empfänger erscheinen automatisch.")
                        .color(MUTED),
                );
            });
            return;
        };
        let title = snapshot.device.name.trim_start_matches("Razer ");
        let title = title.split(" (").next().unwrap_or(title);
        ui.label(Text::new(title).size(32.0).strong().color(TEXT));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if let Some(battery) = self.status.battery {
                readout(
                    ui,
                    if self.status.charging {
                        "Akku · lädt"
                    } else {
                        "Akku"
                    },
                    format!("{battery} %"),
                    ACID,
                );
            }
            if let Some((x, y)) = self.status.dpi {
                let dpi = if x == y || y == 0 {
                    format!("{x} DPI")
                } else {
                    format!("{x} × {y} DPI")
                };
                readout(ui, "Sensor", dpi, VIOLET);
            }
            if let Some(polling) = self.status.polling {
                readout(ui, "Pollingrate", format!("{polling} Hz"), TEXT);
            }
        });
        ui.add_space(16.0);
        if ui.available_width() >= 690.0 {
            ui.columns_const::<2, _>(|[settings, preview]| {
                self.settings_view(settings);
                self.device_view(preview);
            });
        } else {
            self.settings_view(ui);
            ui.add_space(14.0);
            self.device_view(ui);
        }
    }

    fn settings_view(&mut self, ui: &mut Ui) {
        ui.label(Text::new(self.page.title()).size(22.0).strong());
        ui.add_space(5.0);
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let free_spin = snapshot
            .fields
            .iter()
            .zip(&self.draft)
            .find(|(field, _)| matches!(field.key, Key::Cycle(_)))
            .is_none_or(|(_, value)| has_free_spin(value));
        let mut touched = false;
        for (index, (field, edit)) in snapshot.fields.iter().zip(&mut self.draft).enumerate() {
            if field.page != self.page || (!free_spin && field.key.assist()) {
                continue;
            }
            ui.push_id(index, |ui| {
                tile(PANEL).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    touched |= editor(ui, field, edit);
                });
            });
            ui.add_space(4.0);
        }
        if self.page == Page::Wheel && !free_spin {
            ui.label(Text::new("Smart Reel und Scrollbeschleunigung sind bei Kombinationen mit Freilauf verfügbar.").color(MUTED));
        }
        if self.page == Page::Power
            && let Some(battery) = self.status.battery
        {
            ui.add(
                egui::ProgressBar::new(f32::from(battery) / 100.0)
                    .fill(ACID)
                    .text(format!("{battery} %")),
            );
        }
        if self.page == Page::Sensor && snapshot.volatile_dpi {
            ui.label(
                Text::new("DPI-Vorschau aktiv · Speichern über Übernehmen")
                    .color(MUTED)
                    .size(11.0),
            );
        }
        if touched {
            self.preview_deadline = Some(Instant::now() + Duration::from_millis(170));
            let message = if self.changes().is_empty() {
                "Gerät bereit"
            } else {
                "Änderungen ausstehend"
            };
            self.notify(message, false);
        }
    }

    fn device_view(&mut self, ui: &mut Ui) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let mut accent = VIOLET;
        if self.page == Page::Lights
            && let Some((_, Value::Light { effect, color, .. })) = self
                .changes()
                .into_iter()
                .rev()
                .find(|(key, _)| matches!(key, Key::Light(_)))
            && effect.is_some_and(|effect| effect.color())
        {
            accent = Color::from_rgb(color[0], color[1], color[2]);
        }
        let caption = match self.page {
            Page::Wheel if snapshot.device.basilisk() => "Umschalten über die belegte Maustaste",
            Page::Lights => "Entwurf der Beleuchtung",
            Page::Receiver => "Statusanzeigen des Empfängers",
            _ => snapshot.device.connection(),
        }
        .to_owned();
        let mut retry = false;
        tile(RAIL).show(ui, |ui| {
            ui.label(Text::new("PRODUKTBILD").size(12.0).monospace().color(MUTED));
            let width = ui.available_width();
            let height = (width * 0.93).clamp(180.0, 348.0);
            let (area, response) =
                ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
            let painter = ui.painter_at(area);
            let center = area.center();
            if let ProductPicture::Ready(texture) = &self.picture {
                let radius = (height * 0.38).min(width * 0.43);
                for i in (0u8..20).rev() {
                    let fraction = f32::from(i) / 19.0;
                    painter.circle_filled(
                        center,
                        radius * (0.45 + fraction),
                        Color::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 3),
                    );
                }
                painter.circle_stroke(
                    center,
                    radius,
                    Stroke::new(1.0, accent.gamma_multiply(0.28)),
                );
                let size = texture.size_vec2();
                let fit = ((area.width() - 10.0) / size.x).min((area.height() - 10.0) / size.y);
                let photo = Rect::from_center_size(center, size * fit.max(0.0));
                painter.image(
                    texture.id(),
                    photo,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color::WHITE,
                );
            } else {
                let status = match &self.picture {
                    ProductPicture::Missing => "Kein Produktbild verfügbar",
                    ProductPicture::Loading => "Produktbild wird geladen …",
                    ProductPicture::Failed(_) => "Produktbild derzeit nicht erreichbar",
                    ProductPicture::Ready(_) => unreachable!(),
                };
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    status,
                    FontId::proportional(13.0),
                    MUTED,
                );
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(Text::new(&caption).color(MUTED).size(12.0));
                if let ProductPicture::Failed(error) = &self.picture {
                    response.on_hover_text(error);
                    retry = ui.small_button("Erneut versuchen").clicked();
                }
            });
        });
        if retry {
            self.request_photo();
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        let dirty = !self.changes().is_empty();
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(Text::new(&self.message).size(12.0).color(if self.problem {
                    WARN
                } else {
                    MUTED
                }))
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let apply = egui::Button::new(Text::new("Übernehmen").strong().color(if dirty {
                    BASE
                } else {
                    MUTED
                }))
                .fill(if dirty { ACID } else { FIELD })
                .corner_radius(8)
                .min_size(vec2(132.0, 37.0));
                if ui
                    .add_enabled(dirty && !self.busy && !self.devices.is_empty(), apply)
                    .clicked()
                {
                    self.apply();
                }
                if ui
                    .add_enabled(
                        dirty,
                        egui::Button::new("Verwerfen").min_size(vec2(92.0, 37.0)),
                    )
                    .clicked()
                {
                    self.execute(Pending::Reload);
                }
                if ui
                    .add_enabled(
                        self.snapshot.is_some() && !self.devices.is_empty(),
                        egui::Button::new("Neu laden").min_size(vec2(90.0, 37.0)),
                    )
                    .clicked()
                {
                    if dirty {
                        self.confirm = Some(Pending::Reload);
                    } else {
                        self.execute(Pending::Reload);
                    }
                }
            });
        });
    }
}

impl eframe::App for App {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.read_replies(context);
        self.read_photos(context);
        if context.input(|input| input.viewport().close_requested()) && !self.ready_to_close {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.busy && self.confirm.is_none() && !self.closing {
                self.ask(Pending::Close);
            }
        }
        let now = Instant::now();
        if !self.busy && !self.closing {
            if self
                .preview_deadline
                .is_some_and(|deadline| deadline <= now)
            {
                self.preview();
            }
            if now >= self.next_scan && !self.scanning {
                self.scanning = true;
                self.next_scan = now + Duration::from_secs(2);
                let _ = self.worker.send.send(Request::Scan);
            }
        }
        let next = self
            .preview_deadline
            .unwrap_or(now + Duration::from_millis(400));
        context.request_repaint_after(
            next.saturating_duration_since(now)
                .min(Duration::from_millis(400)),
        );
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let enabled = !self.busy && !self.closing && self.confirm.is_none();
        ui.add_enabled_ui(enabled, |ui| {
            egui::Panel::top("brand")
                .exact_size(68.0)
                .frame(panel(RAIL, 16))
                .show(ui, |ui| self.header(ui));
            egui::Panel::bottom("actions")
                .exact_size(63.0)
                .frame(panel(RAIL, 12))
                .show(ui, |ui| self.footer(ui));
            egui::Panel::left("navigation")
                .exact_size(185.0)
                .frame(panel(RAIL, 12))
                .show(ui, |ui| self.sidebar(ui));
            egui::CentralPanel::default()
                .frame(panel(BASE, 18))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.main(ui));
                });
        });
        if self.confirm.is_some() {
            egui::Window::new("Änderungen verwerfen?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label("Deine Entwürfe wurden noch nicht übernommen.");
                    ui.horizontal(|ui| {
                        if ui.button("Weiter bearbeiten").clicked() {
                            self.confirm = None;
                        }
                        if ui.button("Verwerfen").clicked()
                            && let Some(pending) = self.confirm.take()
                        {
                            self.execute(pending);
                        }
                    });
                });
        }
        if self
            .capture
            .as_ref()
            .is_some_and(|(_, when)| *when <= Instant::now())
            && self.snapshot.is_some()
            && !matches!(self.picture, ProductPicture::Loading)
            && let Some((path, _)) = self.capture.take()
        {
            ui.ctx().request_screenshot(move |frame| {
                let data: Vec<u8> = frame
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                if let Err(error) = image::save_buffer(
                    path,
                    &data,
                    frame.size[0] as u32,
                    frame.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("Screenshot: {error}");
                }
            });
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.ready_to_close {
            let _ = self.worker.send.send(Request::Close);
            // On an external renderer shutdown the worker still gets the chance to restore a preview.
        }
    }
}

fn editor(ui: &mut Ui, field: &Field, value: &mut Value) -> bool {
    let before = value.clone();
    let label = match field.key {
        Key::Brightness(Some(zone)) => format!("{} · Helligkeit", zone.name),
        _ => field.label.into(),
    };
    ui.label(Text::new(label).size(14.0).strong().color(TEXT));
    match (&field.control, &mut *value) {
        (Control::Stages { max }, Value::Stages { active, points }) => {
            stages(ui, *max, active, points)
        }
        (
            Control::Dpi {
                max,
                x_only,
                presets,
            },
            Value::Dpi(pair),
        ) => {
            if !presets.is_empty() {
                let options: Vec<_> = presets
                    .iter()
                    .map(|n| ((*n).into(), format!("{n} DPI")))
                    .collect();
                let mut speed = pair.0.into();
                options_row(ui, &options, &mut speed);
                pair.0 = speed as u16;
            } else {
                ui.horizontal_wrapped(|ui| {
                    axis(ui, "DPI X", &mut pair.0, *max);
                    if !x_only {
                        axis(ui, "DPI Y", &mut pair.1, *max);
                    }
                });
            }
        }
        (Control::Switch, Value::Bool(enabled)) => {
            ui.horizontal(|ui| {
                for (on, label) in [(false, "Aus"), (true, "An")] {
                    if button(ui, label, *enabled == on, (68.0, 36.0)).clicked() {
                        *enabled = on;
                    }
                }
            });
        }
        (Control::Choices(options), Value::Number(number)) => options_row(ui, options, number),
        (Control::Range { min, max, unit }, Value::Number(number)) => {
            ui.add(
                egui::Slider::new(number, *min..=*max)
                    .clamping(egui::SliderClamping::Edits)
                    .suffix(format!(" {unit}")),
            );
        }
        (
            Control::Effects(effects),
            Value::Light {
                effect,
                color,
                direction,
            },
        ) => {
            ui.horizontal_wrapped(|ui| {
                for item in effects {
                    if button(ui, item.title(), Some(*item) == *effect, (62.0, 36.0)).clicked() {
                        *effect = Some(*item);
                    }
                }
            });
            if effect.is_some_and(|item| item.color()) {
                ui.horizontal(|ui| {
                    ui.label("Farbe");
                    ui.color_edit_button_srgb(color);
                    ui.label(
                        Text::new(format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]))
                            .monospace()
                            .color(MUTED),
                    );
                });
            }
            if effect.is_some_and(|item| item.direction()) {
                options_row(ui, &[(1, "Rechts".into()), (2, "Links".into())], direction);
            }
        }
        (Control::Indicators(options), Value::Indicators(values)) => {
            for (index, value) in values.iter_mut().enumerate() {
                ui.label(Text::new(format!("LED {}", index + 1)).color(MUTED));
                options_row(ui, options, value);
            }
        }
        _ => (),
    }
    *value != before
}

fn stages(ui: &mut Ui, max: u16, active: &mut u8, points: &mut Vec<Dpi>) {
    ui.add_space(3.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(Text::new("Anzahl").color(MUTED));
        if ui
            .add_enabled(
                points.len() > 1,
                egui::Button::new("−").min_size(vec2(32.0, 32.0)),
            )
            .on_hover_text("Letzte DPI-Stufe entfernen")
            .clicked()
        {
            points.pop();
            *active = (*active).min(points.len() as u8);
        }
        ui.label(
            Text::new(points.len().to_string())
                .size(17.0)
                .monospace()
                .strong(),
        );
        if ui
            .add_enabled(
                points.len() < 5,
                egui::Button::new("+").min_size(vec2(32.0, 32.0)),
            )
            .on_hover_text("DPI-Stufe hinzufügen")
            .clicked()
        {
            points.push(points.last().copied().unwrap_or((800, 800)));
        }
    });
    ui.add_space(5.0);
    ui.label(Text::new("Aktive Stufe").color(MUTED));
    ui.horizontal_wrapped(|ui| {
        for index in 1..=points.len() as u8 {
            let dpi = points[usize::from(index - 1)].0;
            let text = format!("{index}  ·  {dpi}");
            if button(ui, &text, index == *active, (68.0, 36.0)).clicked() {
                *active = index;
            }
        }
    });
    ui.add_space(5.0);
    egui::Grid::new("dpi-axes")
        .spacing(vec2(12.0, 6.0))
        .show(ui, |ui| {
            for label in ["Stufe", "DPI X", "DPI Y"] {
                ui.label(Text::new(label).size(11.0).monospace().color(MUTED));
            }
            ui.end_row();
            for (index, (x, y)) in points.iter_mut().enumerate() {
                let selected = index + 1 == usize::from(*active);
                ui.label(
                    Text::new(format!("{:02}", index + 1))
                        .monospace()
                        .color(if selected { ACID } else { MUTED }),
                );
                dpi_number(ui, x, max);
                dpi_number(ui, y, max);
                ui.end_row();
            }
        });
}

fn dpi_number(ui: &mut Ui, value: &mut u16, max: u16) {
    ui.add_sized(
        [102.0, 34.0],
        egui::DragValue::new(value)
            .range(100..=max.max(100))
            .clamp_existing_to_range(false)
            .speed(10.0),
    )
    .on_hover_text("Klicken zum Eingeben oder ziehen zum Ändern");
}

fn axis(ui: &mut Ui, name: &str, value: &mut u16, max: u16) {
    ui.label(Text::new(name).color(MUTED));
    dpi_number(ui, value, max);
}

fn options_row(ui: &mut Ui, options: &[(i32, String)], number: &mut i32) {
    if !options.iter().any(|(value, _)| value == number) {
        ui.label(
            Text::new(if *number < 0 {
                "Kombination wählen".into()
            } else {
                format!("Aktuell · {number}")
            })
            .color(MUTED),
        );
    }
    ui.horizontal_wrapped(|ui| {
        for (value, label) in options {
            if button(ui, label, value == number, (58.0, 35.0)).clicked() {
                *number = *value;
            }
        }
    });
}

fn theme(context: &egui::Context) {
    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BASE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.window_fill = RAIL;
    style.visuals.window_stroke = Stroke::new(1.0, LINE);
    style.visuals.extreme_bg_color = FIELD;
    style.visuals.selection.bg_fill = VIOLET_FILL;
    style.visuals.selection.stroke = Stroke::new(1.0, VIOLET);
    for state in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        state.bg_fill = FIELD;
        state.weak_bg_fill = FIELD;
        state.bg_stroke = Stroke::new(1.0, LINE);
        state.fg_stroke = Stroke::new(1.0, TEXT);
        state.corner_radius = 7.into();
    }
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, VIOLET);
    style.visuals.widgets.active.bg_fill = VIOLET_FILL;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, VIOLET);
    style.spacing.item_spacing = vec2(7.0, 6.0);
    style.spacing.button_padding = vec2(9.0, 6.0);
    style.spacing.interact_size.y = 31.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    context.set_theme(egui::Theme::Dark);
    context.set_style_of(egui::Theme::Dark, style);
}

fn panel(color: Color, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(color).inner_margin(margin)
}

fn tile(color: Color) -> egui::Frame {
    panel(color, 12)
        .corner_radius(11)
        .stroke(Stroke::new(1.0, LINE))
}

fn button(ui: &mut Ui, title: &str, selected: bool, (width, height): (f32, f32)) -> egui::Response {
    let (fill, stroke, ink) = if selected {
        (VIOLET_FILL, VIOLET, TEXT)
    } else {
        (FIELD, LINE, MUTED)
    };
    ui.add(
        egui::Button::new(Text::new(title).color(ink).strong())
            .selected(selected)
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke))
            .corner_radius(7)
            .min_size(vec2(width, height)),
    )
}

fn tag(ui: &mut Ui, label: &str, color: Color) {
    egui::Frame::new()
        .fill(VIOLET_FILL)
        .corner_radius(6)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.label(Text::new(label).color(color).size(12.0).monospace());
        });
}

fn readout(ui: &mut Ui, title: &str, value: String, color: Color) {
    tile(PANEL).show(ui, |ui| {
        ui.set_min_width(104.0);
        ui.label(Text::new(title).size(11.0).monospace().color(MUTED));
        ui.label(
            Text::new(value)
                .size(17.0)
                .monospace()
                .color(color)
                .strong(),
        );
    });
}

fn logo(painter: &egui::Painter, rect: Rect) {
    let center = rect.center();
    painter.rect_filled(rect, 10.0, VIOLET_FILL);
    painter.line_segment(
        [center + vec2(-10.0, 8.0), center + vec2(0.0, -9.0)],
        Stroke::new(3.0, VIOLET),
    );
    painter.line_segment(
        [center + vec2(0.0, -9.0), center + vec2(9.0, 8.0)],
        Stroke::new(3.0, VIOLET),
    );
    painter.circle_filled(center + vec2(0.0, 4.0), 2.6, ACID);
}
