use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::Result;
use zbus::blocking::{Connection, Proxy, connection::Builder};

use crate::model::Page;

pub const APP_ID: &str = "io.github.ashrynsilkstone_ui.Pointrill";
const WINDOW_PATH: &str = "/io/github/ashrynsilkstone_ui/Pointrill";

struct Window(Sender<Option<Page>>);

#[zbus::interface(name = "io.github.ashrynsilkstone_ui.Pointrill")]
impl Window {
    fn show(&self, view: &str) -> zbus::fdo::Result<()> {
        let page = match view {
            "" => None,
            "profiles" => Some(Page::Profiles),
            "diagnostics" => Some(Page::Diagnostics),
            _ => return Err(zbus::fdo::Error::InvalidArgs("Unbekannte Ansicht".into())),
        };
        self.0
            .send(page)
            .map_err(|_| zbus::fdo::Error::Failed("Pointrill wird gerade beendet".into()))
    }
}

pub struct WindowInstance {
    pub connection: Connection,
    pub requests: Receiver<Option<Page>>,
}

pub fn main_window(page: Option<Page>) -> Result<Option<WindowInstance>> {
    let view = match page {
        Some(Page::Profiles) => "profiles",
        Some(Page::Diagnostics) => "diagnostics",
        _ => "",
    };
    let (sender, receiver) = mpsc::channel();
    let connection = Builder::session()?
        .method_timeout(Duration::from_secs(2))
        .serve_at(WINDOW_PATH, Window(sender))?
        .name(APP_ID)?
        .replace_existing_names(false)
        .build();
    match connection {
        Ok(connection) => Ok(Some(WindowInstance {
            connection,
            requests: receiver,
        })),
        Err(zbus::Error::NameTaken) => {
            let bus = Builder::session()?
                .method_timeout(Duration::from_secs(2))
                .build()?;
            let window = Proxy::new(&bus, APP_ID, WINDOW_PATH, APP_ID)?;
            window.call::<_, _, ()>("Show", &(view,))?;
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}
