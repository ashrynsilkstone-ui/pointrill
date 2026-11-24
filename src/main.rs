mod bus;
mod diagnostics;
mod hid;
mod model;
mod polling;
mod profiles;
mod ui;
mod worker;

use eframe::egui;

fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--list-profiles", serial, pid] => {
            println!(
                "{}",
                serde_json::to_string(&worker::tray_profiles(
                    serial,
                    u16::from_str_radix(pid, 16)?
                )?)?
            );
            return Ok(());
        }
        ["--apply-profile", serial, pid, id] => {
            println!(
                "{}",
                worker::apply_tray_profile(serial, u16::from_str_radix(pid, 16)?, id)?
            );
            return Ok(());
        }
        ["--auto-profile-tick"] => {
            println!("{}", serde_json::to_string(&worker::automatic_tick()?)?);
            return Ok(());
        }
        _ => {}
    }
    if arguments.iter().any(|argument| argument == "--probe") {
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
    let page = match arguments.first().map(String::as_str) {
        Some("--profiles") => model::Page::Profiles,
        Some("--diagnostics") => model::Page::Diagnostics,
        _ => model::Page::Sensor,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1260.0, 800.0])
            .with_min_inner_size([820.0, 580.0])
            .with_app_id("io.github.ashrynsilkstone_ui.Pointrill"),
        ..Default::default()
    };
    eframe::run_native(
        "Pointrill",
        options,
        Box::new(move |context| Ok(Box::new(ui::App::new(&context.egui_ctx, capture, page)))),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}
