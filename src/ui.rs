use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, AtomExt as _, Color32 as Color, FontId, Pos2, Rect, RichText as Text, Stroke, Ui, Vec2,
    vec2,
};

use crate::model::{
    Control, Device, Dpi, Effect, Field, Key, Page, Snapshot, Status, Value, draft_changes,
    has_free_spin,
};
use crate::photos::Loader;
use crate::worker::{Reply, Request, Worker};

const BASE: Color = Color::from_rgb(10, 12, 11);
const RAIL: Color = Color::from_rgb(15, 17, 16);
const PANEL: Color = Color::from_rgb(20, 23, 21);
const FIELD: Color = Color::from_rgb(29, 33, 30);
const LINE: Color = Color::from_rgb(57, 64, 59);
const TEXT: Color = Color::from_rgb(244, 247, 244);
const MUTED: Color = Color::from_rgb(169, 180, 173);
const GREEN: Color = Color::from_rgb(122, 241, 43);
const ACTIVE_BG: Color = Color::from_rgb(30, 54, 20);
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
    library: Vec<crate::profiles::Profile>,
    profile_id: Option<String>,
    profile_name: String,
    profile_program: String,
    profile_file: String,
    profile_review: Option<(String, Vec<String>)>,
    diagnostics: String,
    daemon_error: Option<String>,
    monitor: Option<crate::diagnostics::InputMonitor>,
    file_picker: Option<Child>,
    profile_revision: Option<SystemTime>,
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
    Profile(crate::profiles::Profile),
    DeleteProfile(crate::profiles::Profile),
}

impl App {
    pub fn new(context: &egui::Context, capture: Option<PathBuf>, page: Page) -> Self {
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
            page,
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
            library: Vec::new(),
            profile_id: None,
            profile_name: String::new(),
            profile_program: String::new(),
            profile_file: String::new(),
            profile_review: None,
            diagnostics: String::new(),
            daemon_error: None,
            monitor: None,
            file_picker: None,
            profile_revision: None,
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
        self.monitor = None;
        self.diagnostics.clear();
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
        self.profile_review = None;
        if !same {
            self.status = Status::default();
        }
        self.set_photo(snapshot.product_image.clone());
        self.diagnostics.clear();
        self.profile_revision = crate::profiles::revision(&snapshot.device);
        self.snapshot = Some(snapshot);
        if self.page == Page::Profiles {
            self.reload_profiles();
        }
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
                            if self.devices != devices || self.daemon_error.is_some() {
                                self.diagnostics.clear();
                            }
                            self.daemon_error = None;
                            self.devices = devices;
                            self.status = status;
                            let current = self.snapshot.as_ref().map(|view| &view.device);
                            if self.devices.is_empty() {
                                self.snapshot = None;
                                self.monitor = None;
                                self.set_photo(None);
                                self.draft.clear();
                                self.notify("Warte auf eine Maus oder einen Empfänger.", false);
                            } else if current != selected.as_ref() || current.is_none() {
                                let target = current
                                    .filter(|device| self.devices.contains(device))
                                    .cloned()
                                    .unwrap_or_else(|| self.devices[0].clone());
                                self.load(target);
                            } else if let Some(current) = current
                                && self.changes().is_empty()
                                && self.profile_revision != crate::profiles::revision(current)
                            {
                                self.load(current.clone());
                            }
                        }
                        Err(error) => {
                            let message = format!("{error:#}");
                            if self.daemon_error.as_ref() != Some(&message) {
                                self.diagnostics.clear();
                            }
                            self.devices.clear();
                            self.monitor = None;
                            self.daemon_error = Some(message.clone());
                            self.status = Status::default();
                            self.notify(message, true);
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
                Reply::LightTest(result) => {
                    self.busy = false;
                    match result {
                        Ok(snapshot) => {
                            self.accept_snapshot(snapshot);
                            self.notify(
                                "LED-Test beendet · Ausgangswerte wiederhergestellt",
                                false,
                            );
                        }
                        Err(error) => self.notify(format!("{error:#}"), true),
                    }
                }
                Reply::ProfileCaptured(result) => {
                    self.busy = false;
                    let result = result.and_then(|profile| {
                        crate::profiles::save(&profile, false)?;
                        Ok(profile)
                    });
                    match result {
                        Ok(profile) => {
                            self.reload_profiles();
                            self.select_profile(&profile);
                            self.notify(format!("{} gespeichert", profile.name), false);
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
            Pending::Profile(profile) => self.preview_profile(profile),
            Pending::DeleteProfile(profile) => match crate::profiles::remove(&profile) {
                Ok(()) => {
                    self.profile_id = None;
                    self.profile_program.clear();
                    self.reload_profiles();
                    self.notify(format!("{} gelöscht", profile.name), false);
                }
                Err(error) => self.notify(format!("{error:#}"), true),
            },
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
        page == Page::Diagnostics
            || (page == Page::Profiles && self.snapshot.is_some())
            || self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.fields.iter().any(|field| field.page == page))
            || (page == Page::Power && self.status.battery.is_some())
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (mark, _) = ui.allocate_exact_size(vec2(38.0, 38.0), egui::Sense::hover());
            logo(ui.painter(), mark);
            ui.separator();
            ui.add_space(3.0);
            ui.label(emphasis("POINTRILL", 27.0));
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
                    connection_tag(ui, device.connection());
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.label(emphasis("GERÄTE", 12.0).color(MUTED));
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
            let response = rail_button(ui, Icon::Mouse, name, current, 46.0);
            if response.clicked() && !current {
                target = Some(device.clone());
            }
        }
        if let Some(target) = target {
            self.ask(Pending::Switch(target));
        }
        {
            ui.add_space(24.0);
            ui.label(emphasis("EINSTELLUNGEN", 12.0).color(MUTED));
            ui.add_space(6.0);
            for page in Page::ALL {
                let icon = match page {
                    Page::Sensor => Icon::Sensor,
                    Page::Wheel => Icon::Mouse,
                    Page::Lights => Icon::Light,
                    Page::Power => Icon::Battery,
                    Page::Receiver => Icon::Receiver,
                    Page::Profiles => Icon::Profiles,
                    Page::Diagnostics => Icon::Pulse,
                };
                if self.has_page(page)
                    && rail_button(ui, icon, page.title(), self.page == page, 39.0).clicked()
                {
                    self.open_page(page);
                }
            }
        }
    }

    fn main(&mut self, ui: &mut Ui) {
        if self.page == Page::Diagnostics {
            self.diagnostics_view(ui);
            return;
        }
        if self.page == Page::Profiles && self.snapshot.is_some() {
            self.profiles_view(ui);
            return;
        }
        if self.snapshot.is_none() {
            ui.add_space(66.0);
            ui.vertical_centered(|ui| {
                ui.label(emphasis("Maus verbinden", 32.0));
                ui.label(
                    Text::new("Unterstützte Mäuse und Empfänger erscheinen automatisch.")
                        .color(MUTED),
                );
            });
            return;
        }
        let width = ui.available_width();
        if width >= 595.0 {
            let gap = 16.0;
            let settings_width = ((width - gap) * 0.55).max(390.0);
            let spacing = ui.spacing().item_spacing.x;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let settings = ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    ui.set_width(settings_width);
                    self.device_settings(ui);
                });
                let photo_height = settings.response.rect.height().clamp(354.0, 590.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    ui.set_width(width - gap - settings_width);
                    self.device_view(ui, photo_height);
                });
            });
        } else {
            self.device_settings(ui);
            ui.add_space(12.0);
            self.device_view(ui, (width * 0.8).clamp(300.0, 455.0));
        }
    }

    fn device_settings(&mut self, ui: &mut Ui) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let compact = compact_height(ui);
        if compact {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().interact_size.y = 27.0;
            ui.spacing_mut().button_padding = vec2(6.0, 4.0);
        }
        let title = snapshot.device.name.trim_start_matches("Razer ");
        let title = title.split(" (").next().unwrap_or(title);
        ui.label(emphasis(title, if compact { 32.0 } else { 40.0 }));
        ui.add_space(if compact { 3.0 } else { 6.0 });
        ui.horizontal_wrapped(|ui| {
            if let Some(battery) = self.status.battery {
                readout(
                    ui,
                    Icon::Battery,
                    "Akku",
                    if self.status.charging {
                        format!("{battery} % · lädt")
                    } else {
                        format!("{battery} %")
                    },
                    GREEN,
                );
            }
            if let Some((x, y)) = self.status.dpi {
                let dpi = if x == y || y == 0 {
                    format!("{x} DPI")
                } else {
                    format!("{x} × {y} DPI")
                };
                readout(ui, Icon::Sensor, "Sensor", dpi, TEXT);
            }
            if let Some(polling) = self.status.polling {
                readout(
                    ui,
                    Icon::Pulse,
                    "Pollingrate",
                    format!("{polling} Hz"),
                    TEXT,
                );
            }
        });
        ui.add_space(if compact { 6.0 } else { 18.0 });
        self.settings_view(ui);
    }

    fn settings_view(&mut self, ui: &mut Ui) {
        ui.label(emphasis(self.page.title().to_uppercase(), 23.0));
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
        let padding = if compact_height(ui) { 9 } else { 12 };
        for (index, (field, edit)) in snapshot.fields.iter().zip(&mut self.draft).enumerate() {
            if field.page != self.page || (!free_spin && field.key.assist()) {
                continue;
            }
            ui.push_id(index, |ui| {
                tile(PANEL).inner_margin(padding).show(ui, |ui| {
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
                    .fill(GREEN)
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

    fn device_view(&mut self, ui: &mut Ui, height: f32) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let mut accent = GREEN;
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
            _ => "Produktbild",
        };
        let mut retry = false;
        tile(BASE).show(ui, |ui| {
            let width = ui.available_width();
            let (area, response) =
                ui.allocate_exact_size(vec2(width, height - 52.0), egui::Sense::hover());
            let painter = ui.painter_at(area);
            let center = area.center();
            if let ProductPicture::Ready(texture) = &self.picture {
                photo_backdrop(&painter, area, accent);
                let size = texture.size_vec2();
                let fit = ((area.width() - 36.0) / size.x).min((area.height() - 36.0) / size.y);
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
                ui.label(Text::new(caption).color(MUTED).size(12.0));
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

    fn open_page(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.monitor = None;
        match page {
            Page::Profiles => self.reload_profiles(),
            Page::Diagnostics => self.refresh_diagnostics(),
            _ => {}
        }
    }

    fn reload_profiles(&mut self) {
        match crate::profiles::list() {
            Ok(list) => {
                if let Some(snapshot) = &self.snapshot {
                    let model = crate::profiles::model(&snapshot.device);
                    let mut compatible = list.iter().filter(|profile| profile.model == model);
                    if !compatible
                        .clone()
                        .any(|profile| self.profile_id.as_deref() == Some(&profile.id()))
                    {
                        if let Some(profile) = compatible.next() {
                            self.select_profile(profile);
                        } else {
                            self.profile_id = None;
                            self.profile_program.clear();
                        }
                    }
                }
                self.library = list;
            }
            Err(error) => self.notify(format!("{error:#}"), true),
        }
    }

    fn select_profile(&mut self, profile: &crate::profiles::Profile) {
        self.profile_id = Some(profile.id());
        self.profile_program = profile.program.clone().unwrap_or_default();
    }

    fn choose_profile_file(&mut self, save: bool) {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let path = if self.profile_file.trim().is_empty() {
            if save {
                home.join("pointrill-profil.json")
            } else {
                home
            }
        } else {
            PathBuf::from(self.profile_file.trim())
        };
        match Command::new("kdialog")
            .arg(if save {
                "--getsavefilename"
            } else {
                "--getopenfilename"
            })
            .arg(path)
            .arg("*.json|Pointrill-Profile (*.json)")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(picker) => self.file_picker = Some(picker),
            Err(_) => self.notify(
                "Dateiauswahl nicht verfügbar. Gib den Pfad direkt ein.",
                false,
            ),
        }
    }

    fn check_profile_file(&mut self) {
        let completed = self
            .file_picker
            .as_mut()
            .is_some_and(|picker| picker.try_wait().is_ok_and(|status| status.is_some()));
        if completed
            && let Some(picker) = self.file_picker.take()
            && let Ok(output) = picker.wait_with_output()
            && output.status.success()
        {
            self.profile_file = String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\n', '\r'])
                .to_string();
        }
    }

    fn preview_profile(&mut self, profile: crate::profiles::Profile) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match profile.plan(snapshot) {
            Ok(plan) => {
                let mut details: Vec<_> = plan
                    .changes
                    .iter()
                    .filter_map(|(key, value)| {
                        let field = snapshot.fields.iter().find(|field| &field.key == key)?;
                        let name = if matches!(key, Key::Brightness(Some(_))) {
                            format!("{} · Helligkeit", field.label)
                        } else {
                            field.label.into()
                        };
                        Some(format!(
                            "{name} · vorher {} · danach {}",
                            crate::profiles::display(field, &field.value),
                            crate::profiles::display(field, value)
                        ))
                    })
                    .collect();
                if details.is_empty() {
                    details.push("Keine Änderung an den verfügbaren Einstellungen.".into());
                }
                if !plan.skipped.is_empty() {
                    details.push(format!(
                        "Nicht verfügbar oder hier ungültig: {}",
                        plan.skipped.join(", ")
                    ));
                }
                self.preview_deadline = None;
                if self.preview_sent.take().is_some() {
                    let _ = self.worker.send.send(Request::Preview(None));
                }
                self.draft = plan.draft;
                self.profile_review = Some((profile.name.clone(), details));
                self.notify(format!("Vorschau · {}", profile.name), false);
            }
            Err(error) => self.notify(format!("{error:#}"), true),
        }
    }

    fn profiles_view(&mut self, ui: &mut Ui) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let model = crate::profiles::model(&snapshot.device);
        let device = snapshot.device.clone();
        let profiles: Vec<_> = self
            .library
            .iter()
            .filter(|p| p.model == model)
            .cloned()
            .collect();
        ui.label(emphasis("Profile", 34.0));
        ui.label(Text::new(&device.name).color(MUTED));
        ui.add_space(13.0);
        tile(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(emphasis("Aktuelle Einstellungen sichern", 18.0));
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.profile_name)
                        .hint_text("Profilname, z. B. Desktop")
                        .desired_width(220.0),
                );
                if ui
                    .add_enabled(
                        !self.profile_name.trim().is_empty() && self.changes().is_empty(),
                        egui::Button::new("Profil speichern"),
                    )
                    .clicked()
                {
                    self.busy = true;
                    self.notify("Lese aktuelle Einstellungen …", false);
                    let _ = self
                        .worker
                        .send
                        .send(Request::CaptureProfile(self.profile_name.clone(), None));
                }
                if ui.button("Letzten Stand wiederherstellen").clicked() {
                    match crate::profiles::last(&device) {
                        Ok(backup) => self.ask(Pending::Profile(backup)),
                        Err(error) => self.notify(error.to_string(), true),
                    }
                }
            });
        });
        ui.add_space(9.0);
        tile(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(emphasis("Gespeicherte Profile", 18.0));
                if ui.small_button("Aktualisieren").clicked() {
                    self.reload_profiles();
                }
            });
            if profiles.is_empty() {
                ui.label(Text::new("Für dieses Mausmodell sind noch keine Profile gespeichert.").color(MUTED));
            }
            ui.horizontal_wrapped(|ui| {
                for profile in &profiles {
                    let selected = self.profile_id.as_ref().is_some_and(|id| *id == profile.id());
                    if button(ui, &profile.name, selected, (76.0, 33.0)).clicked() {
                        self.select_profile(profile);
                    }
                }
            });
            if let Some(profile) = profiles.iter().find(|p| self.profile_id.as_ref().is_some_and(|id| *id == p.id())) {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Änderungen ansehen").clicked() {
                        self.ask(Pending::Profile(profile.clone()));
                    }
                    if ui.button("Profil löschen").clicked() {
                        self.confirm = Some(Pending::DeleteProfile(profile.clone()));
                    }
                });
                ui.add_space(6.0);
                ui.label(emphasis("Spielzuordnung", 15.0));
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.profile_program)
                        .hint_text("Programmdatei, z. B. cs2").desired_width(220.0));
                    if ui.button("Zuordnung speichern").clicked() {
                        match profile.automatic(&profile.name, Some(&self.profile_program))
                            .and_then(|updated| crate::profiles::save(&updated, true))
                        {
                            Ok(()) => {
                                self.reload_profiles();
                                self.notify("Spielzuordnung gespeichert", false);
                            }
                            Err(error) => self.notify(format!("{error:#}"), true),
                        }
                    }
                });
                ui.label(Text::new("Der KDE-Tray wechselt beim Start des Programms. Ein leeres Feld deaktiviert die Zuordnung.").color(MUTED));
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.profile_file)
                    .hint_text("Dateipfad zur .json-Datei").desired_width(290.0));
                if ui.add_enabled(self.file_picker.is_none(), egui::Button::new("Datei auswählen …")).clicked() {
                    self.choose_profile_file(false);
                }
                if ui.add_enabled(self.file_picker.is_none(), egui::Button::new("Exportpfad …")).clicked() {
                    self.choose_profile_file(true);
                }
                if ui.add_enabled(!self.profile_file.trim().is_empty(), egui::Button::new("Importieren")).clicked() {
                    let result = crate::profiles::import(Path::new(self.profile_file.trim()))
                        .and_then(|mut profile| {
                            profile.program = None;
                            crate::profiles::save(&profile, false)?;
                            Ok(profile)
                        });
                    match result {
                        Ok(profile) => {
                            self.reload_profiles();
                            self.select_profile(&profile);
                            self.notify(format!("{} importiert", profile.name), false);
                        }
                        Err(error) => self.notify(format!("{error:#}"), true),
                    }
                }
                if let Some(profile) = profiles.iter().find(|p| self.profile_id.as_ref().is_some_and(|id| *id == p.id()))
                    && ui.add_enabled(!self.profile_file.trim().is_empty(), egui::Button::new("Exportieren")).clicked()
                {
                    match crate::profiles::export(profile, Path::new(self.profile_file.trim())) {
                        Ok(()) => self.notify("Profil exportiert", false),
                        Err(error) => self.notify(format!("{error:#}"), true),
                    }
                }
            });
        });
        if let Some((name, rows)) = &self.profile_review {
            ui.add_space(10.0);
            tile(PANEL).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(emphasis(format!("Vorschau · {name}"), 18.0).color(GREEN));
                for row in rows {
                    ui.label(row);
                }
            });
        }
    }

    fn refresh_diagnostics(&mut self) {
        self.diagnostics = crate::diagnostics::report(
            &self.devices,
            self.snapshot.as_ref(),
            self.daemon_error.as_deref(),
        );
    }

    fn diagnostics_view(&mut self, ui: &mut Ui) {
        ui.label(emphasis("Gerätetest", 34.0));
        ui.add_space(8.0);
        if self.diagnostics.is_empty() {
            self.refresh_diagnostics();
        }
        tile(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(emphasis("Geräte und Zugriff", 18.0));
                if ui.button("Neu prüfen").clicked() {
                    self.refresh_diagnostics();
                }
                if ui.button("Diagnose kopieren").clicked() {
                    ui.ctx().copy_text(self.diagnostics.clone());
                    self.notify("Diagnose kopiert", false);
                }
            });
            ui.label(Text::new(&self.diagnostics).monospace().size(13.0));
        });
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let device = snapshot.device.clone();
        let zones: Vec<_> = snapshot
            .fields
            .iter()
            .filter_map(|field| match (&field.key, &field.control) {
                (Key::Light(zone), Control::Effects(effects))
                    if effects.contains(&Effect::Static)
                        && field.validates(&field.value).is_ok()
                        && snapshot
                            .fields
                            .iter()
                            .any(|candidate| candidate.key == Key::Brightness(Some(*zone))) =>
                {
                    Some(*zone)
                }
                _ => None,
            })
            .collect();
        ui.add_space(12.0);
        tile(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(emphasis("Tasten und Scrollrad", 18.0));
            if let Some((x, y)) = self.status.dpi {
                let dpi = if x == y || y == 0 {
                    format!("{x} DPI")
                } else {
                    format!("{x} × {y} DPI")
                };
                ui.label(
                    self.status
                        .dpi_stage
                        .map_or(dpi.clone(), |stage| format!("DPI-Stufe {stage} · {dpi}")),
                );
            }
            if self.monitor.is_none() && ui.button("Eingabetest starten").clicked() {
                self.monitor = Some(crate::diagnostics::InputMonitor::start(&device));
            }
            if self.monitor.is_some() && ui.button("Eingabetest beenden").clicked() {
                self.monitor = None;
            }
            if let Some(monitor) = &mut self.monitor {
                ui.label(Text::new(&monitor.message).color(MUTED));
                if monitor.fallback {
                    let (area, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), 62.0),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(area, 6, FIELD);
                    ui.painter().text(
                        area.center(),
                        egui::Align2::CENTER_CENTER,
                        "Hier klicken und scrollen",
                        FontId::proportional(15.0),
                        TEXT,
                    );
                    ui.input(|input| {
                        for event in &input.events {
                            match event {
                                egui::Event::PointerButton {
                                    pos,
                                    button,
                                    pressed: true,
                                    ..
                                } if area.contains(*pos) => {
                                    let label = match button {
                                        egui::PointerButton::Primary => "Linke Maustaste",
                                        egui::PointerButton::Secondary => "Rechte Maustaste",
                                        egui::PointerButton::Middle => "Mittlere Maustaste",
                                        egui::PointerButton::Extra1 => "Seitentaste 1",
                                        egui::PointerButton::Extra2 => "Seitentaste 2",
                                    };
                                    monitor.record(label.into());
                                }
                                egui::Event::MouseWheel { delta, .. }
                                    if response.hovered() && *delta != Vec2::ZERO =>
                                {
                                    let horizontal = delta.x.abs() > delta.y.abs();
                                    monitor.record(crate::diagnostics::scroll_event(
                                        horizontal,
                                        if horizontal {
                                            delta.x > 0.0
                                        } else {
                                            delta.y > 0.0
                                        },
                                    ));
                                }
                                _ => {}
                            }
                        }
                    });
                }
                for event in &monitor.events {
                    ui.label(event);
                }
            }
        });
        if !zones.is_empty() {
            ui.add_space(12.0);
            tile(PANEL).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(emphasis("LED-Zonen prüfen", 18.0));
                ui.label(Text::new("Die Zone leuchtet zwei Sekunden. Danach stellt Pointrill ihre Einstellung wieder her.").color(MUTED));
                let enabled = self.changes().is_empty();
                ui.horizontal_wrapped(|ui| {
                    for zone in zones {
                        if ui.add_enabled(enabled, egui::Button::new(format!("{} testen", zone.name))).clicked() {
                            self.busy = true;
                            self.notify(format!("Teste {} …", zone.name), false);
                            let _ = self.worker.send.send(Request::TestLight(zone));
                        }
                    }
                });
            });
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        let dirty = !self.changes().is_empty();
        ui.horizontal(|ui| {
            let color = if self.problem {
                WARN
            } else if self.snapshot.is_some() {
                GREEN
            } else {
                MUTED
            };
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.0, color);
            let status_width = (ui.available_width() - 375.0).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(status_width, 36.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            Text::new(&self.message).size(13.0).color(if self.problem {
                                WARN
                            } else {
                                MUTED
                            }),
                        )
                        .truncate(),
                    );
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let apply = egui::Button::new(emphasis("Übernehmen", 15.0).color(if dirty {
                    BASE
                } else {
                    MUTED
                }))
                .fill(if dirty { GREEN } else { FIELD })
                .corner_radius(6)
                .min_size(vec2(132.0, 36.0));
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
        self.check_profile_file();
        if let Some(monitor) = &mut self.monitor {
            monitor.poll();
            context.request_repaint_after(Duration::from_millis(60));
        }
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
        let compact = compact_height(ui);
        ui.add_enabled_ui(enabled, |ui| {
            egui::Panel::top("brand")
                .exact_size(if compact { 58.0 } else { 68.0 })
                .frame(panel(RAIL, if compact { 10 } else { 16 }))
                .show(ui, |ui| self.header(ui));
            egui::Panel::bottom("actions")
                .exact_size(if compact { 56.0 } else { 63.0 })
                .frame(panel(RAIL, if compact { 9 } else { 12 }))
                .show(ui, |ui| self.footer(ui));
            egui::Panel::left("navigation")
                .exact_size(185.0)
                .frame(panel(RAIL, 12))
                .show(ui, |ui| self.sidebar(ui));
            egui::CentralPanel::default()
                .frame(panel(BASE, if compact { 14 } else { 18 }))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.main(ui));
                });
        });
        if let Some(pending) = &self.confirm {
            let (title, detail, remove) = match pending {
                Pending::DeleteProfile(profile) => (
                    "Profil löschen?",
                    format!("„{}“ aus der Sammlung entfernen?", profile.name),
                    true,
                ),
                _ => (
                    "Änderungen verwerfen?",
                    "Deine Entwürfe wurden noch nicht übernommen.".into(),
                    false,
                ),
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label(detail);
                    ui.horizontal(|ui| {
                        if ui
                            .button(if remove {
                                "Abbrechen"
                            } else {
                                "Weiter bearbeiten"
                            })
                            .clicked()
                        {
                            self.confirm = None;
                        }
                        if ui
                            .button(if remove { "Löschen" } else { "Verwerfen" })
                            .clicked()
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
    ui.label(emphasis(label.to_uppercase(), 15.0));
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
    let height = ui.spacing().interact_size.y;
    ui.add_space(3.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(Text::new("Anzahl").color(MUTED));
        if ui
            .add_enabled(
                points.len() > 1,
                egui::Button::new("-").min_size(vec2(30.0, height)),
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
                egui::Button::new("+").min_size(vec2(30.0, height)),
            )
            .on_hover_text("DPI-Stufe hinzufügen")
            .clicked()
        {
            points.push(points.last().copied().unwrap_or((800, 800)));
        }
    });
    ui.add_space(7.0);
    for (index, (x, y)) in points.iter_mut().enumerate() {
        let stage = (index + 1) as u8;
        let selected = stage == *active;
        egui::Frame::new()
            .fill(if selected { ACTIVE_BG } else { RAIL })
            .stroke(Stroke::new(1.0, if selected { GREEN } else { LINE }))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(7, 4))
            .show(ui, |ui| {
                let width = ui.available_width();
                let stage_width = 67.0;
                let axis_label_width = 28.0;
                let gaps = 4.0 * ui.spacing().item_spacing.x;
                let axis_width = ((width - stage_width - 2.0 * axis_label_width - gaps) / 2.0)
                    .clamp(55.0, 158.0);
                ui.set_min_width(width);
                ui.horizontal(|ui| {
                    if button(
                        ui,
                        &format!("Stufe {stage}"),
                        selected,
                        (stage_width, height),
                    )
                    .on_hover_text(format!("DPI-Stufe {stage} aktivieren"))
                    .clicked()
                    {
                        *active = stage;
                    }
                    for (name, number) in [("DPI X", x), ("DPI Y", y)] {
                        let label = ui.add_sized(
                            vec2(axis_label_width, height),
                            egui::Label::new(Text::new(name).size(12.0).color(MUTED)),
                        );
                        dpi_number(ui, number, max, axis_width).labelled_by(label.id);
                    }
                });
            });
    }
}

fn dpi_number(ui: &mut Ui, value: &mut u16, max: u16, width: f32) -> egui::Response {
    ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::DragValue::new(value)
            .range(100..=max.max(100))
            .clamp_existing_to_range(false)
            .speed(10.0),
    )
    .on_hover_text("Klicken zum Eingeben oder ziehen zum Ändern")
}

fn axis(ui: &mut Ui, name: &str, value: &mut u16, max: u16) {
    let label = ui.label(Text::new(name).color(MUTED));
    dpi_number(ui, value, max, 102.0).labelled_by(label.id);
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
    let size = if compact_height(ui) {
        (50.0, 29.0)
    } else {
        (58.0, 35.0)
    };
    ui.horizontal_wrapped(|ui| {
        for (value, label) in options {
            if button(ui, label, value == number, size).clicked() {
                *number = *value;
            }
        }
    });
}

fn theme(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Barlow Regular".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/barlow/Barlow-Regular.ttf"))
            .into(),
    );
    fonts.font_data.insert(
        "Barlow Bold".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/barlow/Barlow-Bold.ttf"))
            .into(),
    );
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    let mut bold = proportional.clone();
    proportional.insert(0, "Barlow Regular".into());
    bold.insert(0, "Barlow Bold".into());
    fonts.families.insert(bold_family(), bold);
    context.set_fonts(fonts);

    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BASE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.window_fill = RAIL;
    style.visuals.window_stroke = Stroke::new(1.0, LINE);
    style.visuals.extreme_bg_color = FIELD;
    style.visuals.selection.bg_fill = ACTIVE_BG;
    style.visuals.selection.stroke = Stroke::new(1.0, GREEN);
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    for state in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        state.bg_fill = FIELD;
        state.weak_bg_fill = FIELD;
        state.bg_stroke = Stroke::new(1.0, LINE);
        state.fg_stroke = Stroke::new(1.0, TEXT);
        state.corner_radius = 6.into();
    }
    style.visuals.widgets.hovered.bg_fill = Color::from_rgb(39, 48, 39);
    style.visuals.widgets.hovered.weak_bg_fill = style.visuals.widgets.hovered.bg_fill;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, GREEN);
    style.visuals.widgets.active.bg_fill = ACTIVE_BG;
    style.visuals.widgets.active.weak_bg_fill = ACTIVE_BG;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, GREEN);
    style.spacing.item_spacing = vec2(7.0, 6.0);
    style.spacing.button_padding = vec2(9.0, 6.0);
    style.spacing.interact_size.y = 31.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    context.set_theme(egui::Theme::Dark);
    context.set_style_of(egui::Theme::Dark, style);
}

fn bold_family() -> egui::FontFamily {
    egui::FontFamily::Name("Barlow Bold".into())
}

fn compact_height(ui: &Ui) -> bool {
    ui.ctx().content_rect().height() < 700.0
}

fn emphasis(text: impl Into<String>, size: f32) -> Text {
    Text::new(text).font(FontId::new(size, bold_family()))
}

fn panel(color: Color, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(color).inner_margin(margin)
}

fn tile(color: Color) -> egui::Frame {
    panel(color, 12)
        .corner_radius(9)
        .stroke(Stroke::new(1.0, LINE))
}

fn button(ui: &mut Ui, title: &str, selected: bool, (width, height): (f32, f32)) -> egui::Response {
    let ink = if selected { BASE } else { TEXT };
    let mut choice = egui::Button::new(emphasis(title, 14.0).color(ink))
        .selected(selected)
        .min_size(vec2(width, height));
    if selected {
        choice = choice.fill(GREEN).stroke(Stroke::new(1.0, GREEN));
    }
    ui.add(choice)
}

fn rail_button(
    ui: &mut Ui,
    glyph: Icon,
    label: &str,
    selected: bool,
    height: f32,
) -> egui::Response {
    let color = if selected { GREEN } else { MUTED };
    let icon = egui::Atom::paint(vec2(22.0, 20.0), move |ui, args| {
        glyph.paint(ui.painter(), args.rect.center(), color);
    });
    let text = emphasis(label, 15.0)
        .atom_grow(true)
        .atom_align(egui::Align2::LEFT_CENTER);
    let mut button = egui::Button::new((icon, text))
        .wrap()
        .selected(selected)
        .min_size(vec2(ui.available_width(), height))
        .frame_when_inactive(false);
    if selected {
        button = button.fill(ACTIVE_BG).stroke(Stroke::new(1.0, GREEN));
    }
    ui.add(button)
}

fn connection_tag(ui: &mut Ui, label: &str) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(19.0, 20.0), egui::Sense::hover());
                Icon::Receiver.paint(ui.painter(), rect.center(), MUTED);
                ui.label(Text::new(label).size(13.0));
            });
        });
}

fn readout(ui: &mut Ui, glyph: Icon, title: &str, value: String, color: Color) {
    tile(PANEL)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::hover());
                glyph.paint(ui.painter(), rect.center(), color);
                if matches!(glyph, Icon::Battery) {
                    ui.label(Text::new(title).size(13.0));
                }
                ui.label(emphasis(value, 19.0).color(color))
                    .on_hover_text(title);
            });
        });
}

#[derive(Clone, Copy)]
enum Icon {
    Battery,
    Light,
    Mouse,
    Pulse,
    Profiles,
    Receiver,
    Sensor,
}

impl Icon {
    fn paint(self, painter: &egui::Painter, center: Pos2, color: Color) {
        let line = Stroke::new(1.5, color);
        let box_outline = |x, y, w, h, corner| {
            painter.rect_stroke(
                Rect::from_center_size(center + vec2(x, y), vec2(w, h)),
                corner,
                line,
                egui::StrokeKind::Inside,
            );
        };
        let segment = |x1, y1, x2, y2| {
            painter.line_segment([center + vec2(x1, y1), center + vec2(x2, y2)], line);
        };
        match self {
            Self::Battery => {
                box_outline(-1.0, 0.0, 16.0, 9.0, 2);
                segment(8.0, -2.0, 8.0, 2.0);
            }
            Self::Light => {
                painter.circle_stroke(center, 4.0, line);
                for step in 0..8 {
                    let angle = step as f32 * std::f32::consts::FRAC_PI_4;
                    let ray = vec2(angle.cos(), angle.sin());
                    painter.line_segment([center + ray * 6.5, center + ray * 8.8], line);
                }
            }
            Self::Mouse => {
                box_outline(0.0, 0.0, 12.0, 17.0, 6);
                segment(0.0, -5.0, 0.0, -1.0);
            }
            Self::Pulse => {
                painter.line(
                    [
                        (-9.0, 0.0),
                        (-5.0, 0.0),
                        (-2.0, -6.0),
                        (2.0, 6.0),
                        (5.0, 0.0),
                        (9.0, 0.0),
                    ]
                    .into_iter()
                    .map(|(x, y)| center + vec2(x, y))
                    .collect(),
                    line,
                );
            }
            Self::Profiles => {
                box_outline(0.0, 0.0, 16.0, 16.0, 2);
                segment(-4.0, -3.0, 4.0, -3.0);
                segment(-4.0, 2.0, 4.0, 2.0);
            }
            Self::Receiver => {
                box_outline(0.0, 3.0, 11.0, 9.0, 2);
                box_outline(0.0, -4.5, 7.0, 6.0, 1);
                segment(-1.0, -4.0, 1.0, -4.0);
            }
            Self::Sensor => {
                painter.circle_stroke(center, 6.2, line);
                for ray in [Vec2::X, Vec2::Y, -Vec2::X, -Vec2::Y] {
                    painter.line_segment([center + ray * 4.3, center + ray * 9.0], line);
                }
                painter.circle_filled(center, 1.3, color);
            }
        }
    }
}

fn photo_backdrop(painter: &egui::Painter, area: Rect, accent: Color) {
    let columns = (area.width() / 23.0).ceil().max(1.0) as u32;
    let rows = (area.height() / 23.0).ceil().max(1.0) as u32;
    let mut glow = egui::Mesh::default();
    for row in 0..=rows {
        for column in 0..=columns {
            let x = column as f32 / columns as f32;
            let y = row as f32 / rows as f32;
            let distance = vec2((x - 0.5) * 2.0, (y - 0.54) * 2.0).length();
            let strength = (1.0 - distance.powi(2)).max(0.0).powi(2);
            let color = Color::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                (65.0 * strength) as u8,
            );
            glow.colored_vertex(area.min + vec2(x * area.width(), y * area.height()), color);
            if row > 0 && column > 0 {
                let bottom_right = row * (columns + 1) + column;
                let top_right = bottom_right - columns - 1;
                glow.add_triangle(top_right - 1, top_right, bottom_right);
                glow.add_triangle(top_right - 1, bottom_right, bottom_right - 1);
            }
        }
    }
    painter.add(egui::Shape::mesh(glow));
    let grid = Stroke::new(1.0, Color::from_white_alpha(7));
    for column in 1..columns {
        let x = area.left() + area.width() * column as f32 / columns as f32;
        painter.line_segment(
            [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
            grid,
        );
    }
    for row in 1..rows {
        let y = area.top() + area.height() * row as f32 / rows as f32;
        painter.line_segment(
            [Pos2::new(area.left(), y), Pos2::new(area.right(), y)],
            grid,
        );
    }
}

fn logo(painter: &egui::Painter, rect: Rect) {
    let center = rect.center();
    painter.line(
        vec![
            center + vec2(-12.0, 10.0),
            center + vec2(0.0, -11.0),
            center + vec2(12.0, 10.0),
        ],
        Stroke::new(3.5, GREEN),
    );
    painter.circle_filled(center + vec2(0.0, 6.0), 3.2, GREEN);
}
