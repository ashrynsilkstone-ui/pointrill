mod bus;
mod hid;
mod model;
mod photos;
mod ui;
mod worker;

use eframe::egui;

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|argument| argument == "--probe") {
        let mut bus = bus::OpenRazer::connect()?;
        let (devices, _) = bus.scan()?;
        for device in devices {
            let (snapshot, _) = bus.describe_fields(device)?;
            let labels: Vec<_> = snapshot.fields.iter().map(|field| field.label).collect();
            println!(
                "{:04X}:{:04X} {} · {} Einstellungen · {}",
                snapshot.device.vid,
                snapshot.device.pid,
                snapshot.device.name,
                labels.len(),
                labels.join(", ")
            );
        }
        return Ok(());
    }
    let capture = std::env::var_os("POINTRILL_SCREENSHOT").map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1190.0, 805.0])
            .with_min_inner_size([820.0, 580.0])
            .with_app_id("io.github.ashrynsilkstone_ui.Pointrill"),
        ..Default::default()
    };
    eframe::run_native(
        "Pointrill",
        options,
        Box::new(move |context| Ok(Box::new(ui::App::new(&context.egui_ctx, capture)))),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}
