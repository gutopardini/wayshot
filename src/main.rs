mod capture;
mod cli;
mod editor;
mod jxl;
mod output;
mod text_editor;
mod ui;

use std::sync::Arc;
use std::sync::mpsc;

use gtk::gio;

const APP_ID: &str = "io.github.gutopardini.wayshot";

fn main() {
    let command = match cli::parse(
        std::env::args_os(),
        std::env::var_os("WAYSHOT_CAPTURE").is_some(),
    ) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("wayshot: {error}");
            eprintln!("Run wayshot --help for usage.");
            std::process::exit(2);
        }
    };
    let cli::CliCommand::Run(options) = command else {
        print!("{}", cli::HELP);
        return;
    };

    if let Some(dir) = &options.save_dir
        && !dir.is_dir()
    {
        eprintln!(
            "wayshot: --path needs an existing directory, got {}",
            dir.display()
        );
        std::process::exit(1);
    }

    gtk::glib::set_prgname(Some(APP_ID));
    gtk::glib::set_application_name("WayShot");

    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("Failed to start async runtime"));
    let startup_capture = options.wants_capture().then(|| {
        let (sender, receiver) = mpsc::channel();
        let mode = options.mode;
        let delay = options.delay.unwrap_or_default();
        runtime.spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let result = capture::capture_screenshot(None, mode).await;
            let _ = sender.send(result);
        });
        receiver
    });

    if options.headless() {
        let receiver = startup_capture.expect("headless capture starts a screenshot");
        let result = receiver.recv().unwrap_or_else(|_| {
            Err(capture::CaptureFailure::failed(
                "The capture task stopped before returning a screenshot.",
            ))
        });
        std::process::exit(output::deliver(&options, result));
    }

    use adw::prelude::*;

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let image = options.image.clone();
    let format = options.format.clone();
    let pending_capture = std::rc::Rc::new(std::cell::RefCell::new(startup_capture));
    app.connect_activate(move |app| {
        let startup_capture = pending_capture.borrow_mut().take();
        ui::build_ui(
            app,
            image.clone(),
            startup_capture,
            runtime.clone(),
            format.clone(),
        );
    });
    app.run_with_args(&[APP_ID]);
}
