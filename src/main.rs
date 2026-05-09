mod editor;
mod ui;

use std::path::PathBuf;

use gtk::gio;

const APP_ID: &str = "io.github.gutopardini.wayshot";

fn main() {
    use adw::prelude::*;

    let mut initial_capture = std::env::var_os("WAYSHOT_CAPTURE").is_some();
    let mut initial_image = None;
    for arg in std::env::args_os().skip(1) {
        if arg == "--capture" || arg == "-c" {
            initial_capture = true;
        } else if initial_image.is_none() {
            initial_image = Some(PathBuf::from(arg));
        }
    }

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        ui::build_ui(app, initial_image.clone(), initial_capture);
    });
    app.run();
}
