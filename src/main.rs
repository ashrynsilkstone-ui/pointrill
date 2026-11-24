mod bus;
mod diagnostics;
mod hid;
mod instance;
mod model;
mod polling;
mod profiles;
mod tray;
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
        ["--tray"] => return tray::run(),
        ["--enable-autostart"] => return tray::change_autostart(true),
        ["--disable-autostart"] => return tray::change_autostart(false),
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
    let requested_page = match arguments.first().map(String::as_str) {
        Some("--profiles") => Some(model::Page::Profiles),
        Some("--diagnostics") => Some(model::Page::Diagnostics),
        _ => None,
    };
    let (_window_connection, requests) = if capture.is_some() {
        (None, None)
    } else {
        match instance::main_window(requested_page) {
            Ok(Some(window)) => (Some(window.connection), Some(window.requests)),
            Ok(None) => return Ok(()),
            Err(error) => {
                eprintln!("Pointrill-Fenstersteuerung: {error:#}");
                (None, None)
            }
        }
    };
    let page = requested_page.unwrap_or(model::Page::Sensor);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1260.0, 800.0])
            .with_min_inner_size([820.0, 580.0])
            .with_app_id(instance::APP_ID),
        ..Default::default()
    };
    eframe::run_native(
        "Pointrill",
        options,
        Box::new(move |context| {
            Ok(Box::new(ui::App::new(
                &context.egui_ctx,
                capture,
                page,
                requests,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}
