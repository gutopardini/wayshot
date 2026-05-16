use std::cell::{Cell, RefCell};
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use ashpd::desktop::ResponseError;
use ashpd::desktop::screenshot::Screenshot;
use ashpd::{Error as PortalError, PortalError as DesktopPortalError};
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::gio;
use gtk::glib;

use crate::APP_ID;
use crate::editor::{self, Annotation, EditorState, Point, Rect, Tool};

const ICON_CAPTURE: &[u8] = include_bytes!("../assets/icons/camera.svg");
const ICON_UNDO: &[u8] = include_bytes!("../assets/icons/arrow-back-up.svg");
const ICON_REDO: &[u8] = include_bytes!("../assets/icons/arrow-forward-up.svg");
const ICON_COPY: &[u8] = include_bytes!("../assets/icons/copy.svg");
const ICON_PALETTE: &[u8] = include_bytes!("../assets/icons/palette.svg");
const ICON_OPEN: &[u8] = include_bytes!("../assets/icons/folder-open.svg");
const ICON_PASTE: &[u8] = include_bytes!("../assets/icons/clipboard.svg");
const ICON_SAVE: &[u8] = include_bytes!("../assets/icons/device-floppy.svg");
const ICON_SETTINGS: &[u8] = include_bytes!("../assets/icons/settings.svg");
const ICON_CLOCK: &[u8] = include_bytes!("../assets/icons/clock.svg");
const ICON_POINTER: &[u8] = include_bytes!("../assets/icons/pointer.svg");
const ICON_ZOOM: &[u8] = include_bytes!("../assets/icons/zoom-in.svg");
const ICON_SELECT: &[u8] = include_bytes!("../assets/icons/select.svg");
const ICON_PEN: &[u8] = include_bytes!("../assets/icons/pencil.svg");
const ICON_ERASER: &[u8] = include_bytes!("../assets/icons/eraser.svg");
const ICON_RECT: &[u8] = include_bytes!("../assets/icons/square.svg");
const ICON_CIRCLE: &[u8] = include_bytes!("../assets/icons/circle.svg");
const ICON_LINE: &[u8] = include_bytes!("../assets/icons/minus.svg");
const ICON_ARROW: &[u8] = include_bytes!("../assets/icons/arrow-right.svg");
const ICON_TEXT: &[u8] = include_bytes!("../assets/icons/text-size.svg");
const ICON_BLUR: &[u8] = include_bytes!("../assets/icons/blur.svg");
const ICON_APP: &[u8] = include_bytes!("../assets/icons/wayshot-icon.svg");

struct CaptureFailure {
    message: String,
    canceled: bool,
}

struct CaptureResponse {
    result: Result<String, CaptureFailure>,
    close_on_cancel: bool,
}

impl From<PortalError> for CaptureFailure {
    fn from(err: PortalError) -> Self {
        let canceled = matches!(
            err,
            PortalError::Response(ResponseError::Cancelled)
                | PortalError::Portal(DesktopPortalError::Cancelled(_))
        );
        Self {
            message: err.to_string(),
            canceled,
        }
    }
}

fn set_image_from_svg(image: &gtk::Image, icon: &[u8], color: &str) {
    let svg = String::from_utf8_lossy(icon).replace("#e6e6e6", color);
    let pixbuf = Pixbuf::from_read(Cursor::new(svg.to_owned().into_bytes()))
        .ok()
        .and_then(|pixbuf| pixbuf.scale_simple(20, 20, gdk_pixbuf::InterpType::Bilinear));
    let paintable = pixbuf.map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
    image.set_paintable(paintable.as_ref());
}

fn create_icon(
    icon: &'static [u8],
    icon_images: &Rc<RefCell<Vec<(gtk::Image, &'static [u8])>>>,
    icon_color: &Rc<RefCell<String>>,
) -> gtk::Image {
    let image = gtk::Image::new();
    let color = {
        let color = icon_color.borrow();
        if color.is_empty() {
            "#e6e6e6".to_string()
        } else {
            color.to_string()
        }
    };
    set_image_from_svg(&image, icon, &color);
    icon_images.borrow_mut().push((image.clone(), icon));
    image
}

fn create_app_title() -> gtk::Box {
    let icon = gtk::Image::new();
    if let Ok(pixbuf) = Pixbuf::from_read(Cursor::new(ICON_APP)) {
        let pixbuf = pixbuf.scale_simple(24, 24, gdk_pixbuf::InterpType::Bilinear);
        let paintable = pixbuf.map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
        icon.set_paintable(paintable.as_ref());
    }
    icon.set_pixel_size(24);

    let label = gtk::Label::builder()
        .label("WayShot")
        .css_classes(["title"])
        .build();

    let title = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .valign(gtk::Align::Center)
        .build();
    title.append(&icon);
    title.append(&label);
    title
}

fn present_window(window: &adw::ApplicationWindow) {
    window.set_visible(true);
    window.unminimize();
    #[allow(deprecated)]
    window.present_with_time(gdk::CURRENT_TIME);
}

fn show_text_editor(
    window: &adw::ApplicationWindow,
    title: &str,
    initial_text: Option<&str>,
    on_submit: impl Fn(String) + 'static,
) {
    let text_window = gtk::Window::builder()
        .transient_for(window)
        .modal(true)
        .title(title)
        .default_width(360)
        .build();

    let text_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let entry = gtk::Entry::builder()
        .placeholder_text("Text")
        .activates_default(true)
        .build();
    if let Some(text) = initial_text {
        entry.set_text(text);
    }
    text_box.append(&entry);

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel_button = gtk::Button::with_label("Cancel");
    let add_button = gtk::Button::with_label("OK");
    add_button.add_css_class("suggested-action");
    actions.append(&cancel_button);
    actions.append(&add_button);
    text_box.append(&actions);
    text_window.set_child(Some(&text_box));

    let on_submit: Rc<dyn Fn(String)> = Rc::new(on_submit);
    let submit = Rc::new({
        let entry = entry.clone();
        let text_window = text_window.clone();
        let on_submit = on_submit.clone();
        move || {
            let text = entry.text().trim().to_string();
            if !text.is_empty() {
                on_submit(text);
            }
            text_window.close();
        }
    });

    {
        let submit = submit.clone();
        add_button.connect_clicked(move |_| submit());
    }
    {
        let submit = submit.clone();
        entry.connect_activate(move |_| submit());
    }
    {
        let text_window = text_window.clone();
        cancel_button.connect_clicked(move |_| text_window.close());
    }

    text_window.present();
    entry.grab_focus();
    entry.select_region(0, -1);
}

fn copy_png_with_wl_copy(pixbuf: &Pixbuf) -> Result<(), String> {
    let png = pixbuf
        .save_to_bufferv("png", &[])
        .map_err(|err| err.to_string())?;
    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open wl-copy stdin.".to_string())?;
    stdin.write_all(&png).map_err(|err| err.to_string())?;
    drop(stdin);
    Ok(())
}

pub fn build_ui(app: &adw::Application, initial_image: Option<PathBuf>, initial_capture: bool) {
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("Failed to start async runtime"));

    let state = Rc::new(RefCell::new(EditorState::new()));
    {
        let mut state = state.borrow_mut();
        state.color = gdk::RGBA::new(1.0, 0.30, 0.30, 1.0);
        state.fit_to_window = true;
        state.zoom = 1.0;
    }

    let icon_images: Rc<RefCell<Vec<(gtk::Image, &'static [u8])>>> =
        Rc::new(RefCell::new(Vec::new()));
    let icon_color = Rc::new(RefCell::new(String::new()));

    if let Some(display) = gdk::Display::default() {
        let app_icon_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/app-icons");
        if app_icon_dir.is_dir() {
            gtk::IconTheme::for_display(&display).add_search_path(app_icon_dir);
        }

        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let style_manager = adw::StyleManager::default();
        let css_provider = css.clone();
        let icon_images_for_theme = icon_images.clone();
        let icon_color_for_theme = icon_color.clone();
        let apply_theme_css = move |is_dark: bool| {
            if is_dark {
                css_provider.load_from_string(
                    ".glass-header { background: rgba(10, 12, 16, 0.72); border-bottom: 1px solid rgba(255,255,255,0.07); box-shadow: 0 10px 28px rgba(0,0,0,0.22); }
                     .tool-palette { background: rgba(18, 21, 27, 0.58); border-radius: 18px; padding: 10px; border: 1px solid rgba(255,255,255,0.14); box-shadow: 0 18px 46px rgba(0,0,0,0.42); }
                     .tool-button { min-width: 40px; min-height: 40px; border-radius: 12px; background: transparent; }
                     .tool-button.toggle:hover { background: rgba(255,255,255,0.10); }
                     .tool-button.toggle:checked { background: rgba(94, 166, 255, 0.22); box-shadow: inset 0 0 0 1px rgba(125,190,255,0.80), 0 0 18px rgba(71,151,255,0.28); }
                     .color-palette { background: rgba(18, 21, 27, 0.54); border-radius: 18px; padding: 9px; border: 1px solid rgba(255,255,255,0.13); box-shadow: 0 16px 42px rgba(0,0,0,0.34); }
                     .color-swatch { min-width: 22px; min-height: 22px; border-radius: 999px; border: 2px solid rgba(255,255,255,0.24); box-shadow: 0 4px 12px rgba(0,0,0,0.18); }
                     .color-swatch.toggle:checked { border: 2px solid rgba(255,255,255,0.95); box-shadow: 0 0 0 2px rgba(94,166,255,0.42); }
                     .color-custom { min-width: 22px; min-height: 22px; border-radius: 999px; border: 2px solid rgba(255,255,255,0.34); background: rgba(255,255,255,0.10); }
                     .color-black { background: #1b1b1b; }
                     .color-white { background: #f5f5f5; }
                     .color-red { background: #ff4d4d; }
                     .color-orange { background: #ff9f1a; }
                     .color-yellow { background: #ffd93d; }
                     .color-green { background: #3ddc84; }
                     .color-blue { background: #3b82f6; }
                     .color-purple { background: #8b5cf6; }
                     .editor-content { background: #090b10; }
                     .editor-scroller { background: transparent; border-radius: 16px; }
                     .editor-canvas { background: #090b10; }
                     .editor-status { color: #edf4ff; font-size: 11px; background: rgba(18,21,27,0.62); border-radius: 999px; padding: 5px 10px; border: 1px solid rgba(255,255,255,0.11); }",
                );
            } else {
                css_provider.load_from_string(
                    ".glass-header { background: rgba(246, 248, 252, 0.78); border-bottom: 1px solid rgba(0,0,0,0.07); box-shadow: 0 10px 28px rgba(44,62,92,0.12); }
                     .tool-palette { background: rgba(248, 250, 255, 0.66); border-radius: 18px; padding: 10px; border: 1px solid rgba(255,255,255,0.70); box-shadow: 0 18px 46px rgba(44,62,92,0.20); }
                     .tool-button { min-width: 40px; min-height: 40px; border-radius: 12px; background: transparent; }
                     .tool-button.toggle:hover { background: rgba(35, 55, 85, 0.08); }
                     .tool-button.toggle:checked { background: rgba(53, 132, 228, 0.16); box-shadow: inset 0 0 0 1px rgba(36,117,211,0.62), 0 0 18px rgba(53,132,228,0.18); }
                     .color-palette { background: rgba(248, 250, 255, 0.66); border-radius: 18px; padding: 9px; border: 1px solid rgba(255,255,255,0.74); box-shadow: 0 16px 42px rgba(44,62,92,0.16); }
                     .color-swatch { min-width: 22px; min-height: 22px; border-radius: 999px; border: 2px solid rgba(255,255,255,0.72); box-shadow: 0 4px 12px rgba(44,62,92,0.16); }
                     .color-swatch.toggle:checked { border: 2px solid rgba(20,35,56,0.72); box-shadow: 0 0 0 2px rgba(53,132,228,0.30); }
                     .color-custom { min-width: 22px; min-height: 22px; border-radius: 999px; border: 2px solid rgba(20,35,56,0.22); background: rgba(255,255,255,0.54); }
                     .color-black { background: #1b1b1b; }
                     .color-white { background: #f5f5f5; }
                     .color-red { background: #ff4d4d; }
                     .color-orange { background: #ff9f1a; }
                     .color-yellow { background: #ffd93d; }
                     .color-green { background: #3ddc84; }
                     .color-blue { background: #3b82f6; }
                     .color-purple { background: #8b5cf6; }
                     .editor-content { background: #eef3f9; }
                     .editor-scroller { background: transparent; border-radius: 16px; }
                     .editor-canvas { background: #eef3f9; }
                     .editor-status { color: #24364f; font-size: 11px; background: rgba(248,250,255,0.72); border-radius: 999px; padding: 5px 10px; border: 1px solid rgba(255,255,255,0.74); }",
                );
            }
            let icon_color_value = if is_dark { "#e6e6e6" } else { "#2b2b2b" };
            *icon_color_for_theme.borrow_mut() = icon_color_value.to_string();
            for (image, icon) in icon_images_for_theme.borrow().iter() {
                set_image_from_svg(image, icon, icon_color_value);
            }
        };
        let initial_dark = style_manager.is_dark();
        apply_theme_css(initial_dark);
        let style_manager_for_notify = style_manager.clone();
        style_manager.connect_dark_notify(move |_| {
            apply_theme_css(style_manager_for_notify.is_dark());
        });
    }

    let header = adw::HeaderBar::builder()
        .title_widget(&create_app_title())
        .build();
    header.add_css_class("glass-header");

    let capture_button = gtk::Button::builder()
        .child(&create_icon(ICON_CAPTURE, &icon_images, &icon_color))
        .tooltip_text("Capture screenshot")
        .build();
    header.pack_start(&capture_button);

    let open_button = gtk::Button::builder()
        .child(&create_icon(ICON_OPEN, &icon_images, &icon_color))
        .tooltip_text("Open image")
        .build();
    let paste_button = gtk::Button::builder()
        .child(&create_icon(ICON_PASTE, &icon_images, &icon_color))
        .tooltip_text("Paste from clipboard")
        .build();
    header.pack_start(&open_button);
    header.pack_start(&paste_button);

    let delay_adjustment = gtk::Adjustment::new(0.0, 0.0, 10.0, 0.5, 1.0, 0.0);
    let delay_spin = gtk::SpinButton::builder()
        .adjustment(&delay_adjustment)
        .digits(1)
        .numeric(true)
        .width_chars(3)
        .tooltip_text("Capture delay (seconds)")
        .build();
    let interactive_toggle = gtk::Switch::builder()
        .tooltip_text("Interactive capture")
        .active(true)
        .build();
    let settings_button = gtk::MenuButton::builder()
        .child(&create_icon(ICON_SETTINGS, &icon_images, &icon_color))
        .tooltip_text("Capture settings")
        .build();
    let settings_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();
    let size_adjustment = gtk::Adjustment::new(4.0, 1.0, 32.0, 1.0, 2.0, 0.0);
    let size_spin = gtk::SpinButton::builder()
        .adjustment(&size_adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .width_chars(2)
        .tooltip_text("Stroke size")
        .build();
    let zoom_adjustment = gtk::Adjustment::new(1.0, 0.25, 3.0, 0.05, 0.1, 0.0);
    let zoom_scale = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .adjustment(&zoom_adjustment)
        .digits(0)
        .draw_value(false)
        .width_request(120)
        .tooltip_text("Zoom")
        .build();
    let fit_toggle = gtk::ToggleButton::with_label("Fit");
    fit_toggle.set_active(true);
    let zoom_reset = gtk::Button::with_label("100%");
    let size_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let size_label = gtk::Label::new(Some("Stroke"));
    size_label.set_xalign(0.0);
    size_label.set_hexpand(true);
    size_row.append(&size_label);
    size_row.append(&size_spin);
    let size_icon = create_icon(ICON_PEN, &icon_images, &icon_color);
    let size_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    size_group.append(&size_icon);
    size_group.append(&size_row);
    let delay_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let delay_label = gtk::Label::new(Some("Delay (s)"));
    delay_label.set_xalign(0.0);
    delay_label.set_hexpand(true);
    delay_row.append(&delay_label);
    delay_row.append(&delay_spin);
    let delay_icon = create_icon(ICON_CLOCK, &icon_images, &icon_color);
    let delay_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    delay_group.append(&delay_icon);
    delay_group.append(&delay_row);
    let interactive_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let interactive_label = gtk::Label::new(Some("Interactive"));
    interactive_label.set_xalign(0.0);
    interactive_label.set_hexpand(true);
    interactive_row.append(&interactive_label);
    interactive_row.append(&interactive_toggle);
    let interactive_icon = create_icon(ICON_POINTER, &icon_images, &icon_color);
    let interactive_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    interactive_group.append(&interactive_icon);
    interactive_group.append(&interactive_row);
    let zoom_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let zoom_label = gtk::Label::new(Some("Zoom"));
    zoom_label.set_xalign(0.0);
    zoom_label.set_hexpand(true);
    zoom_row.append(&zoom_label);
    zoom_row.append(&zoom_scale);
    let zoom_icon = create_icon(ICON_ZOOM, &icon_images, &icon_color);
    let zoom_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    zoom_group.append(&zoom_icon);
    zoom_group.append(&zoom_row);
    let zoom_actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    zoom_actions.append(&fit_toggle);
    zoom_actions.append(&zoom_reset);
    let divider1 = gtk::Separator::new(gtk::Orientation::Horizontal);
    let divider2 = gtk::Separator::new(gtk::Orientation::Horizontal);
    let divider3 = gtk::Separator::new(gtk::Orientation::Horizontal);
    settings_box.append(&size_group);
    settings_box.append(&divider1);
    settings_box.append(&zoom_group);
    settings_box.append(&divider2);
    settings_box.append(&delay_group);
    settings_box.append(&interactive_group);
    settings_box.append(&divider3);
    settings_box.append(&zoom_actions);
    let settings_popover = gtk::Popover::new();
    settings_popover.set_child(Some(&settings_box));
    settings_button.set_popover(Some(&settings_popover));

    header.pack_end(&settings_button);

    let undo_button = gtk::Button::builder()
        .child(&create_icon(ICON_UNDO, &icon_images, &icon_color))
        .tooltip_text("Undo")
        .build();
    let redo_button = gtk::Button::builder()
        .child(&create_icon(ICON_REDO, &icon_images, &icon_color))
        .tooltip_text("Redo")
        .build();
    let copy_button = gtk::Button::builder()
        .child(&create_icon(ICON_COPY, &icon_images, &icon_color))
        .tooltip_text("Copy to clipboard")
        .build();
    let save_button = gtk::Button::builder()
        .child(&create_icon(ICON_SAVE, &icon_images, &icon_color))
        .tooltip_text("Save as PNG")
        .build();
    header.pack_end(&copy_button);
    header.pack_end(&save_button);
    header.pack_end(&redo_button);
    header.pack_end(&undo_button);

    let status = gtk::Label::builder().label("").xalign(0.0).build();
    status.add_css_class("editor-status");
    status.add_css_class("dim-label");
    status.set_visible(false);

    let set_status = Rc::new({
        let status = status.clone();
        move |msg: &str| {
            status.set_text(msg);
            status.set_visible(!msg.is_empty());
        }
    });

    let drawing_area = gtk::DrawingArea::builder()
        .content_width(900)
        .content_height(600)
        .build();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    drawing_area.add_css_class("editor-canvas");

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&drawing_area)
        .build();
    scroller.add_css_class("editor-scroller");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.add_css_class("editor-content");

    content.append(&status);
    content.append(&scroller);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&content));

    let palette = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(16)
        .margin_bottom(16)
        .build();
    palette.add_css_class("tool-palette");

    let color_palette = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::End)
        .margin_start(16)
        .margin_bottom(16)
        .build();
    color_palette.add_css_class("color-palette");

    let color_dialog = Rc::new(gtk::ColorDialog::new());
    let color_button = gtk::Button::builder()
        .child(&create_icon(ICON_PALETTE, &icon_images, &icon_color))
        .tooltip_text("Custom color")
        .build();
    color_button.add_css_class("color-custom");

    let make_tool_button = |icon: &'static [u8], tooltip: &str| {
        let image = create_icon(icon, &icon_images, &icon_color);
        let button = gtk::ToggleButton::builder().child(&image).build();
        button.add_css_class("tool-button");
        button.set_tooltip_text(Some(tooltip));
        button
    };

    let tool_buttons: Vec<(Tool, gtk::ToggleButton)> = vec![
        (Tool::Select, make_tool_button(ICON_SELECT, "Select")),
        (Tool::Pen, make_tool_button(ICON_PEN, "Pen")),
        (Tool::Eraser, make_tool_button(ICON_ERASER, "Eraser")),
        (Tool::Rect, make_tool_button(ICON_RECT, "Rectangle")),
        (Tool::Circle, make_tool_button(ICON_CIRCLE, "Circle")),
        (Tool::Line, make_tool_button(ICON_LINE, "Line")),
        (Tool::Arrow, make_tool_button(ICON_ARROW, "Arrow")),
        (Tool::Text, make_tool_button(ICON_TEXT, "Text")),
        (Tool::Blur, make_tool_button(ICON_BLUR, "Blur")),
    ];

    for (_, button) in tool_buttons.iter() {
        palette.append(button);
    }

    overlay.add_overlay(&palette);
    overlay.add_overlay(&color_palette);

    let toolbar_view = adw::ToolbarView::builder().content(&overlay).build();
    toolbar_view.add_top_bar(&header);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1400)
        .default_height(900)
        .icon_name(APP_ID)
        .title("WayShot")
        .content(&toolbar_view)
        .build();

    window.maximize();

    let zoom_updating = Rc::new(Cell::new(false));
    let fit_updating = Rc::new(Cell::new(false));

    let apply_background = {
        let drawing_area = drawing_area.clone();
        let state = state.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let fit_toggle = fit_toggle.clone();
        let zoom_updating = zoom_updating.clone();
        let fit_updating = fit_updating.clone();
        Rc::new(move |pixbuf: gdk_pixbuf::Pixbuf| {
            let width = pixbuf.width();
            let height = pixbuf.height();
            drawing_area.set_content_width(width);
            drawing_area.set_content_height(height);
            {
                let mut state = state.borrow_mut();
                state.set_background(pixbuf);
                state.fit_to_window = true;
                state.zoom = 1.0;
            }
            fit_updating.set(true);
            fit_toggle.set_active(true);
            fit_updating.set(false);
            zoom_updating.set(true);
            zoom_adjustment.set_value(1.0);
            zoom_updating.set(false);
            drawing_area.queue_draw();
        })
    };

    if !initial_capture {
        present_window(&window);
    }

    if let Some(path) = initial_image {
        match gdk_pixbuf::Pixbuf::from_file(&path) {
            Ok(pixbuf) => {
                apply_background(pixbuf);
                let msg = format!("Opened image: {}", path.display());
                set_status(&msg);
            }
            Err(err) => {
                let msg = format!("Failed to open image: {err}");
                set_status(&msg);
            }
        }
    }

    let (sender, receiver) = mpsc::channel::<CaptureResponse>();

    let set_status_for_timer = set_status.clone();
    let button_for_timer = capture_button.clone();
    let apply_background_for_timer = apply_background.clone();
    let window_for_timer = window.clone();

    glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(response) = receiver.try_recv() {
            button_for_timer.set_sensitive(true);
            match response.result {
                Ok(uri) => {
                    let msg = format!("Captured: {uri}");
                    set_status_for_timer(&msg);
                    let file = gio::File::for_uri(&uri);
                    match file.path() {
                        Some(path) => match gdk_pixbuf::Pixbuf::from_file(path) {
                            Ok(pixbuf) => {
                                apply_background_for_timer(pixbuf);
                            }
                            Err(err) => {
                                let msg = format!("Failed to load image: {err}");
                                set_status_for_timer(&msg);
                            }
                        },
                        None => {
                            set_status_for_timer("Failed to resolve capture path.");
                        }
                    }
                }
                Err(err) => {
                    if response.close_on_cancel && err.canceled {
                        window_for_timer.close();
                        continue;
                    }
                    let msg = format!("Capture failed: {}", err.message);
                    set_status_for_timer(&msg);
                }
            }
            present_window(&window_for_timer);
        }
        glib::ControlFlow::Continue
    });

    let start_capture = Rc::new({
        let runtime = runtime.clone();
        let set_status_for_capture = set_status.clone();
        let button = capture_button.clone();
        let delay_spin = delay_spin.clone();
        let interactive_toggle = interactive_toggle.clone();
        let window_for_capture = window.clone();
        let skip_hide_once = Rc::new(Cell::new(initial_capture));
        move || {
            let close_on_cancel = skip_hide_once.replace(false);
            let hide_before_capture = !close_on_cancel;
            button.set_sensitive(false);
            set_status_for_capture("Capturing via portal...");
            if hide_before_capture {
                window_for_capture.minimize();
                window_for_capture.set_visible(false);
            }

            let runtime = runtime.clone();
            let sender = sender.clone();
            let delay = delay_spin.value();
            let interactive = interactive_toggle.is_active();

            runtime.spawn(async move {
                if hide_before_capture {
                    let hide_delay = std::time::Duration::from_millis(200);
                    tokio::time::sleep(hide_delay).await;
                }
                if delay > 0.0 {
                    tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                }
                let result = Screenshot::request()
                    .interactive(interactive)
                    .modal(true)
                    .send()
                    .await
                    .and_then(|request| request.response())
                    .map(|response| response.uri().to_string())
                    .map_err(CaptureFailure::from);

                let _ = sender.send(CaptureResponse {
                    result,
                    close_on_cancel,
                });
            });
        }
    });

    {
        let start_capture = start_capture.clone();
        capture_button.connect_clicked(move |_| {
            start_capture();
        });
    }

    if initial_capture {
        let start_capture = start_capture.clone();
        glib::idle_add_local_once(move || {
            start_capture();
        });
    }

    let state_for_draw = state.clone();
    let draw_area_for_draw = drawing_area.clone();
    drawing_area.set_draw_func(move |_, ctx, width, height| {
        {
            let mut state = state_for_draw.borrow_mut();
            state.viewport_width = width;
            state.viewport_height = height;
            if let Some(background) = state.background.as_ref() {
                let (scale, _, _) = editor::view_transform(&state);
                let scaled_w = (background.width() as f64 * scale).round() as i32;
                let scaled_h = (background.height() as f64 * scale).round() as i32;
                let content_w = scaled_w.max(1);
                let content_h = scaled_h.max(1);
                if draw_area_for_draw.content_width() != content_w {
                    draw_area_for_draw.set_content_width(content_w);
                }
                if draw_area_for_draw.content_height() != content_h {
                    draw_area_for_draw.set_content_height(content_h);
                }
            }
        }
        let state = state_for_draw.borrow();
        editor::draw(&state, ctx);
    });

    let move_changed = Rc::new(Cell::new(false));
    let eraser_changed = Rc::new(Cell::new(false));
    let drag = gtk::GestureDrag::new();
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let move_changed = move_changed.clone();
        let eraser_changed = eraser_changed.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut state = state.borrow_mut();
            let point_view = Point { x, y };
            let point = editor::map_to_image(&state, x, y);
            state.drag_start_view = Some(point_view);
            state.drag_last_image = Some(point);
            move_changed.set(false);
            eraser_changed.set(false);
            match state.tool {
                Tool::Select => {
                    state.selected = editor::hit_test(&state.annotations, point);
                    if state.selected.is_some() {
                        state.draft = None;
                    }
                }
                Tool::Pen => {
                    state.draft = Some(Annotation::Pen {
                        points: vec![point],
                        color: state.color,
                        width: state.stroke_width,
                    });
                }
                Tool::Eraser => {
                    if editor::hit_test(&state.annotations, point).is_some() {
                        state.record_change();
                        eraser_changed.set(state.erase_at(point));
                    }
                }
                Tool::Rect => {
                    state.draft = Some(Annotation::Rect {
                        rect: Rect {
                            x1: point.x,
                            y1: point.y,
                            x2: point.x,
                            y2: point.y,
                        },
                        color: state.color,
                        width: state.stroke_width,
                    });
                }
                Tool::Circle => {
                    state.draft = Some(Annotation::Circle {
                        rect: Rect {
                            x1: point.x,
                            y1: point.y,
                            x2: point.x,
                            y2: point.y,
                        },
                        color: state.color,
                        width: state.stroke_width,
                    });
                }
                Tool::Line | Tool::Arrow => {
                    state.draft = Some(Annotation::Line {
                        start: point,
                        end: point,
                        color: state.color,
                        width: state.stroke_width,
                        arrow: matches!(state.tool, Tool::Arrow),
                    });
                }
                Tool::Blur => {
                    state.draft = Some(Annotation::Blur {
                        rect: Rect {
                            x1: point.x,
                            y1: point.y,
                            x2: point.x,
                            y2: point.y,
                        },
                        pixel_size: 10,
                    });
                }
                Tool::Text => {
                    state.draft = None;
                }
            }
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let move_changed = move_changed.clone();
        let eraser_changed = eraser_changed.clone();
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            let mut state = state.borrow_mut();
            let Some(start) = state.drag_start_view else {
                return;
            };
            let current_view = Point {
                x: start.x + offset_x,
                y: start.y + offset_y,
            };
            let current = editor::map_to_image(&state, current_view.x, current_view.y);
            match state.tool {
                Tool::Select => {
                    if let Some(index) = state.selected {
                        if let Some(last) = state.drag_last_image {
                            let dx = current.x - last.x;
                            let dy = current.y - last.y;
                            if !move_changed.get() && (dx.abs() > 0.0 || dy.abs() > 0.0) {
                                state.record_change();
                                move_changed.set(true);
                            }
                            if let Some(annotation) = state.annotations.get_mut(index) {
                                editor::move_annotation(annotation, dx, dy);
                                state.drag_last_image = Some(current);
                            }
                        }
                    }
                }
                Tool::Eraser => {
                    if editor::hit_test(&state.annotations, current).is_some() {
                        if !eraser_changed.get() {
                            state.record_change();
                            eraser_changed.set(true);
                        }
                        state.erase_at(current);
                    }
                }
                _ => match state.draft.as_mut() {
                    Some(Annotation::Pen { points, .. }) => {
                        points.push(current);
                    }
                    Some(Annotation::Rect { rect, .. }) => {
                        rect.x2 = current.x;
                        rect.y2 = current.y;
                    }
                    Some(Annotation::Circle { rect, .. }) => {
                        rect.x2 = current.x;
                        rect.y2 = current.y;
                    }
                    Some(Annotation::Line { end, .. }) => {
                        *end = current;
                    }
                    Some(Annotation::Blur { rect, .. }) => {
                        rect.x2 = current.x;
                        rect.y2 = current.y;
                    }
                    _ => {}
                },
            }
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let move_changed = move_changed.clone();
        drag.connect_drag_end(move |_, offset_x, offset_y| {
            {
                let mut state = state.borrow_mut();
                if let Some(start) = state.drag_start_view.take() {
                    let end_view = Point {
                        x: start.x + offset_x,
                        y: start.y + offset_y,
                    };
                    let end = editor::map_to_image(&state, end_view.x, end_view.y);
                    match state.tool {
                        Tool::Select => {
                            if !move_changed.get() {
                                state.discard_unchanged_record();
                            }
                            state.drag_last_image = None;
                        }
                        _ => {
                            if let Some(mut draft) = state.draft.take() {
                                match &mut draft {
                                    Annotation::Line { end: line_end, .. } => *line_end = end,
                                    Annotation::Rect { rect, .. } => {
                                        rect.x2 = end.x;
                                        rect.y2 = end.y;
                                    }
                                    Annotation::Circle { rect, .. } => {
                                        rect.x2 = end.x;
                                        rect.y2 = end.y;
                                    }
                                    Annotation::Blur { rect, .. } => {
                                        rect.x2 = end.x;
                                        rect.y2 = end.y;
                                    }
                                    Annotation::Pen { points, .. } => {
                                        points.push(end);
                                    }
                                    _ => {}
                                }
                                state.push_annotation(draft);
                            }
                        }
                    }
                }
            }
            drawing_area.queue_draw();
        });
    }
    drawing_area.add_controller(drag);

    let click = gtk::GestureClick::new();
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let window = window.clone();
        click.connect_pressed(move |_, n_press, x, y| {
            let pos = {
                let state = state.borrow();
                editor::map_to_image(&state, x, y)
            };
            let tool = state.borrow().tool;
            match tool {
                Tool::Text => {
                    let (color, size) = {
                        let state = state.borrow();
                        (state.color, state.text_size)
                    };
                    show_text_editor(&window, "Add Text", None, {
                        let state = state.clone();
                        let drawing_area = drawing_area.clone();
                        move |text| {
                            state.borrow_mut().push_annotation(Annotation::Text {
                                pos,
                                text,
                                color,
                                size,
                            });
                            drawing_area.queue_draw();
                        }
                    });
                }
                Tool::Select => {
                    let mut editor_state = state.borrow_mut();
                    editor_state.selected = editor::hit_test(&editor_state.annotations, pos);
                    drawing_area.queue_draw();

                    if n_press == 2 {
                        if let Some(index) = editor_state.selected {
                            if let Some(Annotation::Text { text, .. }) =
                                editor_state.annotations.get(index).cloned()
                            {
                                drop(editor_state);
                                show_text_editor(&window, "Edit Text", Some(&text), {
                                    let state = state.clone();
                                    let drawing_area = drawing_area.clone();
                                    move |new_text| {
                                        let mut editor_state = state.borrow_mut();
                                        let changed = matches!(
                                            editor_state.annotations.get(index),
                                            Some(Annotation::Text { text, .. }) if *text != new_text
                                        );
                                        if !changed {
                                            return;
                                        }
                                        editor_state.record_change();
                                        if let Some(Annotation::Text { text, .. }) =
                                            editor_state.annotations.get_mut(index)
                                        {
                                            *text = new_text;
                                        }
                                        editor_state.selected = Some(index);
                                        drawing_area.queue_draw();
                                    }
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        });
    }
    drawing_area.add_controller(click);

    {
        let buttons = Rc::new(tool_buttons);
        let state = state.clone();

        for (tool, button) in buttons.iter() {
            let tool = *tool;
            let buttons = buttons.clone();
            let state = state.clone();
            button.connect_toggled(move |active_button| {
                if !active_button.is_active() {
                    return;
                }
                for (_, other) in buttons.iter() {
                    if other != active_button {
                        other.set_active(false);
                    }
                }
                let mut state = state.borrow_mut();
                state.tool = tool;
                state.draft = None;
                state.selected = None;
            });
        }

        for (tool, button) in buttons.iter() {
            if *tool == Tool::Pen {
                button.set_active(true);
                break;
            }
        }
    }

    {
        let colors: Vec<(&str, gdk::RGBA)> = vec![
            ("color-black", gdk::RGBA::new(0.11, 0.11, 0.11, 1.0)),
            ("color-white", gdk::RGBA::new(0.96, 0.96, 0.96, 1.0)),
            ("color-red", gdk::RGBA::new(1.0, 0.30, 0.30, 1.0)),
            ("color-orange", gdk::RGBA::new(1.0, 0.62, 0.10, 1.0)),
            ("color-yellow", gdk::RGBA::new(1.0, 0.85, 0.24, 1.0)),
            ("color-green", gdk::RGBA::new(0.24, 0.86, 0.52, 1.0)),
            ("color-blue", gdk::RGBA::new(0.23, 0.51, 0.96, 1.0)),
            ("color-purple", gdk::RGBA::new(0.55, 0.36, 0.96, 1.0)),
        ];

        let buttons: Vec<(gdk::RGBA, gtk::ToggleButton)> = colors
            .iter()
            .map(|(class, color)| {
                let button = gtk::ToggleButton::builder().build();
                button.add_css_class("color-swatch");
                button.add_css_class(class);
                button.set_tooltip_text(Some(&class["color-".len()..]));
                (*color, button)
            })
            .collect();

        for (_, button) in buttons.iter() {
            color_palette.append(button);
        }
        color_palette.append(&color_button);

        let buttons = Rc::new(buttons);
        let state = state.clone();

        for (color, button) in buttons.iter() {
            let color = *color;
            let buttons = buttons.clone();
            let state = state.clone();
            button.connect_toggled(move |active_button| {
                if !active_button.is_active() {
                    return;
                }
                for (_, other) in buttons.iter() {
                    if other != active_button {
                        other.set_active(false);
                    }
                }
                state.borrow_mut().color = color;
            });
        }

        for (color, button) in buttons.iter() {
            if (color.red() - 1.0).abs() < 0.001 && (color.green() - 0.30).abs() < 0.001 {
                button.set_active(true);
                break;
            }
        }
    }
    {
        let state = state.clone();
        let window = window.clone();
        let dialog = color_dialog.clone();
        color_button.connect_clicked(move |_| {
            let current = state.borrow().color;
            dialog.choose_rgba(Some(&window), Some(&current), None::<&gio::Cancellable>, {
                let state = state.clone();
                move |result| {
                    if let Ok(color) = result {
                        state.borrow_mut().color = color;
                    }
                }
            });
        });
    }
    {
        let state = state.clone();
        size_spin.connect_value_changed(move |spin| {
            state.borrow_mut().stroke_width = spin.value();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let undo_action = gio::SimpleAction::new("undo", None);
        {
            let state = state.clone();
            let drawing_area = drawing_area.clone();
            undo_action.connect_activate(move |_, _| {
                state.borrow_mut().undo();
                drawing_area.queue_draw();
            });
        }
        window.add_action(&undo_action);
        app.set_accels_for_action("win.undo", &["<Control>z"]);

        let redo_action = gio::SimpleAction::new("redo", None);
        {
            let state = state.clone();
            let drawing_area = drawing_area.clone();
            redo_action.connect_activate(move |_, _| {
                state.borrow_mut().redo();
                drawing_area.queue_draw();
            });
        }
        window.add_action(&redo_action);
        app.set_accels_for_action("win.redo", &["<Control>y"]);

        {
            let state = state.clone();
            let drawing_area = drawing_area.clone();
            undo_button.connect_clicked(move |_| {
                state.borrow_mut().undo();
                drawing_area.queue_draw();
            });
        }
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        redo_button.connect_clicked(move |_| {
            state.borrow_mut().redo();
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let key_controller = gtk::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Delete && key != gdk::Key::BackSpace {
                return glib::Propagation::Proceed;
            }

            let mut state = state.borrow_mut();
            if let Some(index) = state.selected.take() {
                if state.remove_annotation(index).is_some() {
                    drawing_area.queue_draw();
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);
    }

    {
        let state = state.clone();
        let set_status = set_status.clone();
        let set_status_for_render = set_status.clone();
        let render_for_copy = Rc::new(move || {
            let state = state.borrow();
            let Some(pixbuf) = editor::render_to_pixbuf(&state) else {
                set_status_for_render("Nothing to copy yet.");
                return None;
            };
            Some(pixbuf)
        });
        let copy_to_gtk_clipboard = Rc::new({
            let set_status = set_status.clone();
            move |pixbuf: &Pixbuf| {
                let texture = gdk::Texture::for_pixbuf(pixbuf);
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_texture(&texture);
                    set_status("Copied to clipboard.");
                    true
                } else {
                    set_status("Clipboard unavailable.");
                    false
                }
            }
        });

        let copy_for_button = {
            let render_for_copy = render_for_copy.clone();
            let copy_to_gtk_clipboard = copy_to_gtk_clipboard.clone();
            move || {
                if let Some(pixbuf) = render_for_copy() {
                    copy_to_gtk_clipboard(&pixbuf);
                }
            }
        };

        let copy_for_close = {
            let render_for_copy = render_for_copy.clone();
            let copy_to_gtk_clipboard = copy_to_gtk_clipboard.clone();
            let set_status = set_status.clone();
            move || {
                let Some(pixbuf) = render_for_copy() else {
                    return false;
                };
                match copy_png_with_wl_copy(&pixbuf) {
                    Ok(()) => true,
                    Err(err) => {
                        copy_to_gtk_clipboard(&pixbuf);
                        let msg = format!("Copied, but keep WayShot open: {err}");
                        set_status(&msg);
                        false
                    }
                }
            }
        };

        let copy_for_button = Rc::new(copy_for_button);
        let copy_for_close = Rc::new(copy_for_close);

        let copy_action = gio::SimpleAction::new("copy", None);
        {
            let copy_for_close = copy_for_close.clone();
            let window = window.clone();
            copy_action.connect_activate(move |_, _| {
                if copy_for_close() {
                    window.close();
                }
            });
        }
        window.add_action(&copy_action);
        app.set_accels_for_action("win.copy", &["<Control>c"]);

        copy_button.connect_clicked(move |_| {
            copy_for_button();
        });
    }

    {
        let window = window.clone();
        let set_status = set_status.clone();
        let apply_background = apply_background.clone();
        let file_dialog = gtk::FileDialog::new();
        file_dialog.set_title("Open Image");
        open_button.connect_clicked(move |_| {
            let apply_background = apply_background.clone();
            let set_status = set_status.clone();
            file_dialog.open(
                Some(&window),
                None::<&gio::Cancellable>,
                move |res| match res {
                    Ok(file) => match file.path() {
                        Some(path) => match gdk_pixbuf::Pixbuf::from_file(path) {
                            Ok(pixbuf) => {
                                apply_background(pixbuf);
                                set_status("Opened image.");
                            }
                            Err(err) => {
                                let msg = format!("Failed to open image: {err}");
                                set_status(&msg);
                            }
                        },
                        None => set_status("Failed to resolve file path."),
                    },
                    Err(err) => {
                        let msg = format!("Open canceled: {err}");
                        set_status(&msg);
                    }
                },
            );
        });
    }

    {
        let set_status = set_status.clone();
        let apply_background = apply_background.clone();
        paste_button.connect_clicked(move |_| {
            if let Some(display) = gdk::Display::default() {
                let clipboard = display.clipboard();
                clipboard.read_texture_async(None::<&gio::Cancellable>, {
                    let set_status = set_status.clone();
                    let apply_background = apply_background.clone();
                    move |res| match res {
                        Ok(Some(texture)) => {
                            #[allow(deprecated)]
                            if let Some(pixbuf) = gdk::pixbuf_get_from_texture(&texture) {
                                apply_background(pixbuf);
                                set_status("Pasted from clipboard.");
                            } else {
                                set_status("Clipboard image unavailable.");
                            }
                        }
                        Ok(None) => set_status("Clipboard has no image."),
                        Err(err) => {
                            let msg = format!("Paste failed: {err}");
                            set_status(&msg);
                        }
                    }
                });
            } else {
                set_status("Clipboard unavailable.");
            }
        });
    }

    {
        let window = window.clone();
        let state = state.clone();
        let set_status = set_status.clone();
        let file_dialog = gtk::FileDialog::new();
        file_dialog.set_title("Save PNG");
        save_button.connect_clicked(move |_| {
            let Some(pixbuf) = editor::render_to_pixbuf(&state.borrow()) else {
                set_status("Nothing to save yet.");
                return;
            };
            let texture = gdk::Texture::for_pixbuf(&pixbuf);
            let set_status = set_status.clone();
            file_dialog.save(
                Some(&window),
                None::<&gio::Cancellable>,
                move |res| match res {
                    Ok(file) => match file.path() {
                        Some(mut path) => {
                            if path.extension().is_none() {
                                path.set_extension("png");
                            }
                            match texture.save_to_png(&path) {
                                Ok(()) => set_status("Saved PNG."),
                                Err(err) => {
                                    let msg = format!("Save failed: {err}");
                                    set_status(&msg);
                                }
                            }
                        }
                        None => set_status("Failed to resolve save path."),
                    },
                    Err(err) => {
                        let msg = format!("Save canceled: {err}");
                        set_status(&msg);
                    }
                },
            );
        });
    }

    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let fit_toggle = fit_toggle.clone();
        let zoom_updating = zoom_updating.clone();
        let fit_updating = fit_updating.clone();
        zoom_adjustment.connect_value_changed(move |adj| {
            if zoom_updating.get() {
                return;
            }
            let was_fit = state.borrow().fit_to_window;
            if was_fit {
                fit_updating.set(true);
                fit_toggle.set_active(false);
                fit_updating.set(false);
            }
            let mut state = state.borrow_mut();
            state.fit_to_window = false;
            state.zoom = adj.value();
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let zoom_updating = zoom_updating.clone();
        let fit_updating = fit_updating.clone();
        fit_toggle.connect_toggled(move |toggle| {
            if fit_updating.get() {
                return;
            }
            let mut state = state.borrow_mut();
            state.fit_to_window = toggle.is_active();
            if state.fit_to_window {
                let (scale, _, _) = editor::view_transform(&state);
                zoom_updating.set(true);
                zoom_adjustment.set_value(scale);
                zoom_updating.set(false);
            }
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let zoom_updating = zoom_updating.clone();
        zoom_reset.connect_clicked(move |_| {
            let mut state = state.borrow_mut();
            state.fit_to_window = false;
            state.zoom = 1.0;
            zoom_updating.set(true);
            zoom_adjustment.set_value(1.0);
            zoom_updating.set(false);
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area_for_scroll = drawing_area.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let zoom_updating = zoom_updating.clone();
        let fit_updating = fit_updating.clone();
        let fit_toggle = fit_toggle.clone();
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |controller, _, dy| {
            if !controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let mut state = state.borrow_mut();
            state.fit_to_window = false;
            fit_updating.set(true);
            fit_toggle.set_active(false);
            fit_updating.set(false);
            let factor = if dy < 0.0 { 1.1 } else { 0.9 };
            state.zoom = (state.zoom * factor).clamp(0.25, 3.0);
            zoom_updating.set(true);
            zoom_adjustment.set_value(state.zoom);
            zoom_updating.set(false);
            drawing_area_for_scroll.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(scroll);
    }
}
