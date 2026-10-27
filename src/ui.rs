use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use ashpd::WindowIdentifier;
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::gio;
use gtk::glib;

use crate::APP_ID;
use crate::capture::{self, CaptureFailure, CaptureMode};
use crate::editor::{self, Annotation, EditorState, Point, Rect, Tool};
use crate::jxl;
use crate::output;
use crate::text_editor::InlineTextEditor;

const ICON_CAPTURE: &[u8] = include_bytes!("../assets/icons/camera.svg");
const ICON_UNDO: &[u8] = include_bytes!("../assets/icons/arrow-back-up.svg");
const ICON_REDO: &[u8] = include_bytes!("../assets/icons/arrow-forward-up.svg");
const ICON_COPY: &[u8] = include_bytes!("../assets/icons/copy.svg");
const ICON_PALETTE: &[u8] = include_bytes!("../assets/icons/palette.svg");
const ICON_OPEN: &[u8] = include_bytes!("../assets/icons/folder-open.svg");
const ICON_OPEN_LOCATION: &[u8] = include_bytes!("../assets/icons/folder-up.svg");
const ICON_PASTE: &[u8] = include_bytes!("../assets/icons/clipboard.svg");
const ICON_SAVE: &[u8] = include_bytes!("../assets/icons/device-floppy.svg");
const ICON_SETTINGS: &[u8] = include_bytes!("../assets/icons/settings.svg");
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
const CAPTURE_RESPONSE_POLL_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy)]
struct SaveFormat {
    id: &'static str,
    label: &'static str,
    extension: &'static str,
    pixbuf_type: &'static str,
}

const SAVE_FORMATS: &[SaveFormat] = &[
    SaveFormat {
        id: "png",
        label: "PNG",
        extension: "png",
        pixbuf_type: "png",
    },
    SaveFormat {
        id: "jpeg",
        label: "JPEG",
        extension: "jpg",
        pixbuf_type: "jpeg",
    },
    SaveFormat {
        id: "webp",
        label: "WebP",
        extension: "webp",
        pixbuf_type: "webp",
    },
    SaveFormat {
        id: "jxl",
        label: "JPEG XL (JXL)",
        extension: "jxl",
        pixbuf_type: "jxl",
    },
    SaveFormat {
        id: "avif",
        label: "AVIF",
        extension: "avif",
        pixbuf_type: "avif",
    },
    SaveFormat {
        id: "tiff",
        label: "TIFF",
        extension: "tiff",
        pixbuf_type: "tiff",
    },
];

struct CaptureResponse {
    result: Result<String, CaptureFailure>,
    close_on_cancel: bool,
}

fn available_save_formats() -> Vec<SaveFormat> {
    let formats = Pixbuf::formats();
    SAVE_FORMATS
        .iter()
        .copied()
        .filter(|save_format| {
            if save_format.id == "jxl" {
                return jxl::can_save();
            }
            formats.iter().any(|format| {
                format.is_writable()
                    && !format.is_disabled()
                    && format
                        .name()
                        .is_some_and(|name| name.as_str() == save_format.pixbuf_type)
            }) && format_can_save(save_format.pixbuf_type)
        })
        .collect()
}

fn format_can_save(pixbuf_type: &str) -> bool {
    let Some(pixbuf) = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 1, 1) else {
        return false;
    };

    pixbuf.save_to_bufferv(pixbuf_type, &[]).is_ok()
}

fn save_format_by_id(id: &str) -> SaveFormat {
    SAVE_FORMATS
        .iter()
        .copied()
        .find(|save_format| save_format.id == id)
        .unwrap_or(SAVE_FORMATS[0])
}

fn set_image_from_svg(image: &gtk::Image, icon: &[u8], color: &str) {
    let svg = String::from_utf8_lossy(icon).replace("#e6e6e6", color);
    let pixbuf = Pixbuf::from_read(Cursor::new(svg.to_owned().into_bytes()))
        .ok()
        .and_then(|pixbuf| pixbuf.scale_simple(20, 20, gdk_pixbuf::InterpType::Bilinear));
    let paintable = pixbuf.map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
    image.set_paintable(paintable.as_ref());
}

type IconImage = (gtk::Image, &'static [u8]);
type IconImages = Rc<RefCell<Vec<IconImage>>>;

fn create_icon(
    icon: &'static [u8],
    icon_images: &IconImages,
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
    window.present();
}

fn theme_css(is_dark: bool) -> &'static str {
    if is_dark {
        r#"
        .glass-header {
            background: linear-gradient(90deg, rgba(6, 16, 24, 0.88), rgba(12, 20, 28, 0.80));
            border-bottom: 1px solid rgba(148, 163, 184, 0.18);
            box-shadow: 0 12px 34px rgba(0, 0, 0, 0.36);
        }
        .header-action {
            min-width: 38px;
            min-height: 38px;
            border-radius: 10px;
            margin: 0 2px;
        }
        .capture-action {
            background: #0d9488;
            box-shadow: inset 0 0 0 1px rgba(153, 246, 228, 0.34), 0 10px 24px rgba(13, 148, 136, 0.32);
        }
        .export-action {
            background: rgba(249, 115, 22, 0.18);
            box-shadow: inset 0 0 0 1px rgba(251, 146, 60, 0.45);
        }
        .editor-content {
            background: radial-gradient(circle at 18% 12%, rgba(20, 184, 166, 0.16), transparent 34%),
                        linear-gradient(135deg, #071015, #0c1117 46%, #111827);
        }
        .editor-scroller {
            background: rgba(2, 6, 12, 0.34);
            border-radius: 16px;
            border: 1px solid rgba(148, 163, 184, 0.16);
            box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.025);
        }
        .editor-canvas { background: transparent; }
        .editor-status {
            color: #e8fffb;
            font-size: 11px;
            background: rgba(8, 18, 26, 0.72);
            border-radius: 999px;
            padding: 6px 11px;
            border: 1px solid rgba(45, 212, 191, 0.28);
            box-shadow: 0 10px 26px rgba(0, 0, 0, 0.26);
        }
        .control-dock, .empty-state, .tools-card {
            background: rgba(8, 18, 26, 0.72);
            border: 1px solid rgba(148, 163, 184, 0.18);
            box-shadow: 0 18px 50px rgba(0, 0, 0, 0.42);
        }
        .tool-palette {
            background: transparent;
            border: none;
            box-shadow: none;
            padding: 0;
            min-width: 52px;
        }
        .tools-card {
            border-radius: 16px;
            padding: 8px;
        }
        .tool-button {
            min-width: 42px;
            min-height: 42px;
            border-radius: 11px;
            background: transparent;
        }
        .tool-button.toggle:hover { background: rgba(45, 212, 191, 0.12); }
        .tool-button.toggle:checked {
            background: rgba(20, 184, 166, 0.26);
            box-shadow: inset 0 0 0 1px rgba(94, 234, 212, 0.72), 0 0 22px rgba(20, 184, 166, 0.28);
        }
        .settings-button {
            background: rgba(255, 255, 255, 0.055);
            box-shadow: inset 0 0 0 1px rgba(148, 163, 184, 0.16);
        }
        .settings-button:hover { background: rgba(45, 212, 191, 0.12); }
        .tool-divider { background: rgba(148, 163, 184, 0.22); margin: 4px 6px; }
        .control-dock {
            border-radius: 16px;
            padding: 8px;
        }
        .dock-section {
            border-radius: 12px;
            padding: 4px 8px;
            background: rgba(255, 255, 255, 0.045);
        }
        .dock-label {
            color: #b7c9c6;
            font-size: 10px;
            font-weight: 700;
        }
        .dock-divider {
            background: rgba(148, 163, 184, 0.20);
            margin: 4px 2px;
        }
        .dock-button {
            border-radius: 9px;
            min-height: 30px;
        }
        .compact-spin {
            min-height: 30px;
        }
        .color-swatch {
            min-width: 24px;
            min-height: 24px;
            border-radius: 999px;
            border: 2px solid rgba(255, 255, 255, 0.24);
            box-shadow: 0 4px 14px rgba(0, 0, 0, 0.22);
        }
        .color-swatch.toggle:checked {
            border: 2px solid rgba(255, 255, 255, 0.98);
            box-shadow:
                0 0 0 2px rgba(2, 6, 12, 0.92),
                0 0 0 4px rgba(94, 234, 212, 0.94),
                0 8px 18px rgba(0, 0, 0, 0.34);
        }
        .color-custom {
            min-width: 26px;
            min-height: 26px;
            border-radius: 999px;
            border: 2px solid rgba(255, 255, 255, 0.34);
            background: rgba(255, 255, 255, 0.10);
        }
        .empty-state {
            border-radius: 18px;
            padding: 28px;
            color: #e8fffb;
        }
        .empty-title {
            font-size: 28px;
            font-weight: 700;
            color: #f8fffe;
        }
        .empty-subtitle {
            font-size: 13px;
            color: #b7c9c6;
        }
        .empty-primary {
            background: #0d9488;
            color: #ffffff;
            border-radius: 10px;
            padding: 8px 14px;
        }
        .empty-secondary {
            border-radius: 10px;
            padding: 8px 14px;
        }
        .color-black { background: #1b1b1b; }
        .color-white { background: #f5f5f5; }
        .color-red { background: #ff4d4d; }
        .color-orange { background: #f97316; }
        .color-yellow { background: #ffd93d; }
        .color-green { background: #14b8a6; }
        .color-blue { background: #3b82f6; }
        .color-purple { background: #8b5cf6; }
        "#
    } else {
        r#"
        .glass-header {
            background: linear-gradient(90deg, rgba(240, 253, 250, 0.92), rgba(248, 250, 252, 0.86));
            border-bottom: 1px solid rgba(15, 23, 42, 0.08);
            box-shadow: 0 12px 34px rgba(15, 23, 42, 0.12);
        }
        .header-action {
            min-width: 38px;
            min-height: 38px;
            border-radius: 10px;
            margin: 0 2px;
        }
        .capture-action {
            background: #0d9488;
            box-shadow: inset 0 0 0 1px rgba(240, 253, 250, 0.50), 0 10px 24px rgba(13, 148, 136, 0.24);
        }
        .export-action {
            background: rgba(249, 115, 22, 0.16);
            box-shadow: inset 0 0 0 1px rgba(249, 115, 22, 0.32);
        }
        .editor-content {
            background: radial-gradient(circle at 18% 12%, rgba(20, 184, 166, 0.18), transparent 34%),
                        linear-gradient(135deg, #f0fdfa, #f8fafc 48%, #e2e8f0);
        }
        .editor-scroller {
            background: rgba(255, 255, 255, 0.52);
            border-radius: 16px;
            border: 1px solid rgba(15, 23, 42, 0.08);
            box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.62);
        }
        .editor-canvas { background: transparent; }
        .editor-status {
            color: #134e4a;
            font-size: 11px;
            background: rgba(240, 253, 250, 0.82);
            border-radius: 999px;
            padding: 6px 11px;
            border: 1px solid rgba(20, 184, 166, 0.30);
            box-shadow: 0 10px 26px rgba(15, 23, 42, 0.10);
        }
        .control-dock, .empty-state, .tools-card {
            background: rgba(255, 255, 255, 0.72);
            border: 1px solid rgba(255, 255, 255, 0.82);
            box-shadow: 0 18px 50px rgba(15, 23, 42, 0.18);
        }
        .tool-palette {
            background: transparent;
            border: none;
            box-shadow: none;
            padding: 0;
            min-width: 52px;
        }
        .tools-card {
            border-radius: 16px;
            padding: 8px;
        }
        .tool-button {
            min-width: 42px;
            min-height: 42px;
            border-radius: 11px;
            background: transparent;
        }
        .tool-button.toggle:hover { background: rgba(15, 118, 110, 0.10); }
        .tool-button.toggle:checked {
            background: rgba(13, 148, 136, 0.18);
            box-shadow: inset 0 0 0 1px rgba(13, 148, 136, 0.58), 0 0 20px rgba(13, 148, 136, 0.18);
        }
        .settings-button {
            background: rgba(15, 23, 42, 0.045);
            box-shadow: inset 0 0 0 1px rgba(15, 23, 42, 0.08);
        }
        .settings-button:hover { background: rgba(15, 118, 110, 0.10); }
        .tool-divider { background: rgba(15, 23, 42, 0.12); margin: 4px 6px; }
        .control-dock {
            border-radius: 16px;
            padding: 8px;
        }
        .dock-section {
            border-radius: 12px;
            padding: 4px 8px;
            background: rgba(15, 23, 42, 0.045);
        }
        .dock-label {
            color: #476663;
            font-size: 10px;
            font-weight: 700;
        }
        .dock-divider {
            background: rgba(15, 23, 42, 0.12);
            margin: 4px 2px;
        }
        .dock-button {
            border-radius: 9px;
            min-height: 30px;
        }
        .compact-spin {
            min-height: 30px;
        }
        .color-swatch {
            min-width: 24px;
            min-height: 24px;
            border-radius: 999px;
            border: 2px solid rgba(255, 255, 255, 0.82);
            box-shadow: 0 4px 14px rgba(15, 23, 42, 0.14);
        }
        .color-swatch.toggle:checked {
            border: 2px solid rgba(255, 255, 255, 0.98);
            box-shadow:
                0 0 0 2px rgba(15, 23, 42, 0.86),
                0 0 0 4px rgba(13, 148, 136, 0.86),
                0 8px 18px rgba(15, 23, 42, 0.18);
        }
        .color-custom {
            min-width: 26px;
            min-height: 26px;
            border-radius: 999px;
            border: 2px solid rgba(19, 78, 74, 0.22);
            background: rgba(255, 255, 255, 0.62);
        }
        .empty-state {
            border-radius: 18px;
            padding: 28px;
            color: #134e4a;
        }
        .empty-title {
            font-size: 28px;
            font-weight: 700;
            color: #0f2f2d;
        }
        .empty-subtitle {
            font-size: 13px;
            color: #476663;
        }
        .empty-primary {
            background: #0d9488;
            color: #ffffff;
            border-radius: 10px;
            padding: 8px 14px;
        }
        .empty-secondary {
            border-radius: 10px;
            padding: 8px 14px;
        }
        .color-black { background: #1b1b1b; }
        .color-white { background: #f5f5f5; }
        .color-red { background: #ff4d4d; }
        .color-orange { background: #f97316; }
        .color-yellow { background: #ffd93d; }
        .color-green { background: #14b8a6; }
        .color-blue { background: #2563eb; }
        .color-purple { background: #8b5cf6; }
        "#
    }
}

pub fn build_ui(
    app: &adw::Application,
    initial_image: Option<PathBuf>,
    startup_capture: Option<mpsc::Receiver<Result<String, CaptureFailure>>>,
    runtime: Arc<tokio::runtime::Runtime>,
    format_override: Option<String>,
) {
    let initial_capture = startup_capture.is_some();

    let state = Rc::new(RefCell::new(EditorState::new()));
    load_settings(&state);
    {
        let mut state = state.borrow_mut();
        state.fit_to_window = true;
        state.zoom = 1.0;
        if let Some(format) = format_override {
            state.save_format = format;
        }
    }

    let icon_images: IconImages = Rc::new(RefCell::new(Vec::new()));
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
            css_provider.load_from_string(theme_css(is_dark));
            let icon_color_value = if is_dark { "#e8fffb" } else { "#134e4a" };
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
    capture_button.add_css_class("header-action");
    capture_button.add_css_class("capture-action");
    header.pack_start(&capture_button);

    let open_button = gtk::Button::builder()
        .child(&create_icon(ICON_OPEN, &icon_images, &icon_color))
        .tooltip_text("Open image")
        .build();
    open_button.add_css_class("header-action");
    let paste_button = gtk::Button::builder()
        .child(&create_icon(ICON_PASTE, &icon_images, &icon_color))
        .tooltip_text("Paste from clipboard")
        .build();
    paste_button.add_css_class("header-action");
    let source_folder_button = gtk::Button::builder()
        .child(&create_icon(ICON_OPEN_LOCATION, &icon_images, &icon_color))
        .tooltip_text("Open containing folder")
        .build();
    source_folder_button.add_css_class("header-action");
    source_folder_button.set_visible(false);
    header.pack_start(&open_button);
    header.pack_start(&paste_button);
    header.pack_start(&source_folder_button);

    let size_adjustment =
        gtk::Adjustment::new(state.borrow().stroke_width, 1.0, 32.0, 1.0, 2.0, 0.0);
    let text_size_adjustment =
        gtk::Adjustment::new(state.borrow().text_size, 8.0, 128.0, 1.0, 4.0, 0.0);
    let size_spin = gtk::SpinButton::builder()
        .adjustment(&size_adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .width_chars(3)
        .tooltip_text("Stroke size")
        .build();
    size_spin.add_css_class("compact-spin");
    let text_size_quick_spin = gtk::SpinButton::builder()
        .adjustment(&text_size_adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .width_chars(3)
        .tooltip_text("Text size")
        .build();
    text_size_quick_spin.add_css_class("compact-spin");
    text_size_quick_spin.set_visible(false);
    let zoom_adjustment = gtk::Adjustment::new(1.0, 0.25, 3.0, 0.05, 0.1, 0.0);
    let zoom_scale = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .adjustment(&zoom_adjustment)
        .digits(0)
        .draw_value(false)
        .width_request(150)
        .tooltip_text("Zoom")
        .build();
    let fit_toggle = gtk::ToggleButton::with_label("Fit");
    fit_toggle.set_active(true);
    fit_toggle.add_css_class("dock-button");
    let zoom_reset = gtk::Button::with_label("100%");
    zoom_reset.add_css_class("dock-button");
    let size_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let size_label = gtk::Label::new(Some("Stroke"));
    size_label.set_xalign(0.0);
    size_label.set_hexpand(true);
    size_label.set_width_chars(9);
    size_label.add_css_class("dock-label");
    size_row.append(&size_label);
    size_row.append(&size_spin);
    size_row.append(&text_size_quick_spin);
    let size_icon = create_icon(ICON_PEN, &icon_images, &icon_color);
    let text_size_icon = create_icon(ICON_TEXT, &icon_images, &icon_color);
    text_size_icon.set_visible(false);
    let size_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    size_group.append(&size_icon);
    size_group.append(&text_size_icon);
    size_group.append(&size_row);
    let zoom_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let zoom_label = gtk::Label::new(Some("Zoom"));
    zoom_label.set_xalign(0.0);
    zoom_label.set_hexpand(true);
    zoom_label.add_css_class("dock-label");
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
    for group in [&size_group, &zoom_group] {
        group.add_css_class("dock-section");
    }
    zoom_actions.add_css_class("dock-section");

    let undo_button = gtk::Button::builder()
        .child(&create_icon(ICON_UNDO, &icon_images, &icon_color))
        .tooltip_text("Undo")
        .build();
    undo_button.add_css_class("header-action");
    let redo_button = gtk::Button::builder()
        .child(&create_icon(ICON_REDO, &icon_images, &icon_color))
        .tooltip_text("Redo")
        .build();
    redo_button.add_css_class("header-action");
    let copy_button = gtk::Button::builder()
        .child(&create_icon(ICON_COPY, &icon_images, &icon_color))
        .tooltip_text("Copy to clipboard")
        .build();
    copy_button.add_css_class("header-action");
    copy_button.add_css_class("export-action");
    let save_button = gtk::Button::builder()
        .child(&create_icon(ICON_SAVE, &icon_images, &icon_color))
        .tooltip_text("Save image")
        .build();
    save_button.add_css_class("header-action");
    header.pack_end(&copy_button);
    header.pack_end(&save_button);
    header.pack_end(&redo_button);
    header.pack_end(&undo_button);

    let status = gtk::Label::builder()
        .label("")
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .build();
    status.add_css_class("editor-status");
    status.add_css_class("dim-label");
    status.set_visible(false);

    let set_status_detail = Rc::new({
        let status = status.clone();
        move |msg: &str, detail: Option<&str>| {
            status.set_text(msg);
            status.set_visible(!msg.is_empty());
            status.set_tooltip_text(detail.filter(|detail| !detail.is_empty()));
        }
    });
    let set_status = Rc::new({
        let set_status_detail = set_status_detail.clone();
        move |msg: &str| set_status_detail(msg, None)
    });

    let source_file = Rc::new(RefCell::new(None::<PathBuf>));
    let set_source_file = Rc::new({
        let source_file = source_file.clone();
        let source_folder_button = source_folder_button.clone();
        move |path: Option<PathBuf>| {
            let tooltip = path
                .as_ref()
                .map(|path| format!("Open containing folder\n{}", path.display()));
            *source_file.borrow_mut() = path;
            source_folder_button.set_visible(source_file.borrow().is_some());
            source_folder_button.set_tooltip_text(tooltip.as_deref());
        }
    });

    {
        let source_file = source_file.clone();
        let set_status = set_status.clone();
        source_folder_button.connect_clicked(move |_| {
            let Some(path) = source_file.borrow().clone() else {
                return;
            };
            let Some(parent) = path.parent() else {
                set_status("Failed to resolve containing folder.");
                return;
            };
            let folder = gio::File::for_path(parent);
            if let Err(err) =
                gio::AppInfo::launch_default_for_uri(&folder.uri(), None::<&gio::AppLaunchContext>)
            {
                let msg = format!("Failed to open containing folder: {err}");
                set_status(&msg);
            }
        });
    }

    let drawing_area = gtk::DrawingArea::builder()
        .content_width(900)
        .content_height(600)
        .focusable(true)
        .build();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    drawing_area.add_css_class("editor-canvas");

    let canvas_overlay = gtk::Overlay::builder().child(&drawing_area).build();
    let text_editor = InlineTextEditor::new(state.clone(), &canvas_overlay, &drawing_area);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&canvas_overlay)
        .build();
    scroller.add_css_class("editor-scroller");
    scroller.set_visible(false);

    let editor_view = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .margin_top(18)
        .margin_bottom(3)
        .margin_end(18)
        .build();

    editor_view.append(&scroller);
    editor_view.append(&status);

    let empty_state = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(420)
        .build();
    empty_state.add_css_class("empty-state");

    let empty_icon = gtk::Image::new();
    if let Ok(pixbuf) = Pixbuf::from_read(Cursor::new(ICON_APP)) {
        let pixbuf = pixbuf.scale_simple(64, 64, gdk_pixbuf::InterpType::Bilinear);
        let paintable = pixbuf.map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
        empty_icon.set_paintable(paintable.as_ref());
    }
    empty_icon.set_pixel_size(64);

    let empty_title = gtk::Label::builder()
        .label("Capture. Mark. Ship.")
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();
    empty_title.add_css_class("empty-title");

    let empty_subtitle = gtk::Label::builder()
        .label("Start with a screenshot, an image file, or your clipboard.")
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();
    empty_subtitle.add_css_class("empty-subtitle");

    let empty_actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .halign(gtk::Align::Center)
        .build();
    let empty_capture_button = gtk::Button::with_label("Capture");
    empty_capture_button.add_css_class("empty-primary");
    let empty_open_button = gtk::Button::with_label("Open");
    empty_open_button.add_css_class("empty-secondary");
    let empty_paste_button = gtk::Button::with_label("Paste");
    empty_paste_button.add_css_class("empty-secondary");
    empty_actions.append(&empty_capture_button);
    empty_actions.append(&empty_open_button);
    empty_actions.append(&empty_paste_button);

    empty_state.append(&empty_icon);
    empty_state.append(&empty_title);
    empty_state.append(&empty_subtitle);
    empty_state.append(&empty_actions);

    let palette = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Fill)
        .vexpand(true)
        .margin_start(3)
        .margin_end(3)
        .margin_top(18)
        .margin_bottom(3)
        .build();
    palette.add_css_class("tool-palette");
    palette.set_visible(false);

    let tools_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    tools_group.add_css_class("tools-card");

    let color_palette = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    color_palette.add_css_class("dock-section");

    let control_dock = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    control_dock.add_css_class("control-dock");
    control_dock.set_visible(false);

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

    for (index, (_, button)) in tool_buttons.iter().enumerate() {
        if matches!(index, 1 | 3 | 7) {
            let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
            divider.add_css_class("tool-divider");
            tools_group.append(&divider);
        }
        tools_group.append(button);
    }
    let tools_spacer_top = gtk::Box::builder().vexpand(true).build();
    let tools_spacer_bottom = gtk::Box::builder().vexpand(true).build();
    palette.append(&tools_spacer_top);
    palette.append(&tools_group);
    palette.append(&tools_spacer_bottom);

    let settings_button = gtk::Button::builder()
        .child(&create_icon(ICON_SETTINGS, &icon_images, &icon_color))
        .tooltip_text("Settings")
        .build();
    settings_button.add_css_class("dock-button");
    settings_button.add_css_class("settings-button");

    let settings_popover = gtk::Popover::new();
    settings_popover.set_has_arrow(false);
    settings_popover.set_parent(&settings_button);
    let settings_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .width_request(260)
        .build();

    let text_size_spin = gtk::SpinButton::builder()
        .adjustment(&text_size_adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .width_chars(3)
        .tooltip_text("Text size")
        .build();
    text_size_spin.add_css_class("compact-spin");
    let text_size_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let text_size_label = gtk::Label::builder()
        .label("Text size")
        .xalign(0.0)
        .hexpand(true)
        .build();
    text_size_row.append(&text_size_label);
    text_size_row.append(&text_size_spin);

    let available_formats = available_save_formats();
    let format_labels: Vec<&str> = available_formats
        .iter()
        .map(|format| format.label)
        .collect();
    let save_format_dropdown = gtk::DropDown::from_strings(&format_labels);
    let saved_format = state.borrow().save_format.clone();
    let active_format_index = available_formats
        .iter()
        .position(|format| format.id == saved_format)
        .unwrap_or(0);
    if !available_formats.is_empty() {
        save_format_dropdown.set_selected(active_format_index as u32);
        let active_format = available_formats[active_format_index];
        if saved_format != active_format.id {
            state.borrow_mut().save_format = active_format.id.to_string();
        }
    }
    let save_format_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let save_format_label = gtk::Label::builder()
        .label("Save format")
        .xalign(0.0)
        .hexpand(true)
        .build();
    save_format_row.append(&save_format_label);
    save_format_row.append(&save_format_dropdown);

    let close_after_copy_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let close_after_copy_label = gtk::Label::builder()
        .label("Close after Ctrl+C copy")
        .xalign(0.0)
        .hexpand(true)
        .build();
    let close_after_copy_switch = gtk::Switch::builder()
        .active(state.borrow().close_after_copy)
        .valign(gtk::Align::Center)
        .build();
    close_after_copy_row.append(&close_after_copy_label);
    close_after_copy_row.append(&close_after_copy_switch);
    settings_box.append(&close_after_copy_row);
    settings_box.append(&text_size_row);
    settings_box.append(&save_format_row);
    settings_popover.set_child(Some(&settings_box));

    let text_size_updating = Rc::new(Cell::new(false));
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let text_size_updating = text_size_updating.clone();
        let text_editor = text_editor.clone();
        text_size_adjustment.connect_value_changed(move |adjustment| {
            if text_size_updating.get() {
                return;
            }

            let value = adjustment.value();
            {
                let mut state = state.borrow_mut();
                state.text_size = value;
                if let Some(index) = state.selected
                    && matches!(
                        state.annotations.get(index),
                        Some(Annotation::Text { size, .. }) if (*size - value).abs() > f64::EPSILON
                    )
                {
                    if state.editing_text() != Some(index) {
                        state.record_change();
                    }
                    if let Some(Annotation::Text { size, .. }) = state.annotations.get_mut(index) {
                        *size = value;
                    }
                }
            }
            save_settings(&state);
            text_editor.refresh();
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let available_formats = available_formats.clone();
        save_format_dropdown.connect_selected_notify(move |dropdown| {
            if let Some(format) = available_formats.get(dropdown.selected() as usize) {
                state.borrow_mut().save_format = format.id.to_string();
                save_settings(&state);
            }
        });
    }

    let sync_size_controls = Rc::new({
        let state = state.clone();
        let text_size_adjustment = text_size_adjustment.clone();
        let text_size_updating = text_size_updating.clone();
        let text_editor = text_editor.clone();
        let size_label = size_label.clone();
        let size_spin = size_spin.clone();
        let text_size_quick_spin = text_size_quick_spin.clone();
        let size_icon = size_icon.clone();
        let text_size_icon = text_size_icon.clone();
        move || {
            let (text_size, show_text_size) = {
                let state = state.borrow();
                let selected = state
                    .selected
                    .and_then(|index| state.annotations.get(index));
                let text_size = match selected {
                    Some(Annotation::Text { size, .. }) => *size,
                    _ => state.text_size,
                };
                let show_text_size = match selected {
                    Some(Annotation::Text { .. }) => true,
                    Some(_) => false,
                    None => state.tool == Tool::Text,
                };
                (text_size, show_text_size)
            };
            text_size_updating.set(true);
            text_size_adjustment.set_value(text_size);
            text_size_updating.set(false);
            size_label.set_text(if show_text_size {
                "Text size"
            } else {
                "Stroke"
            });
            size_spin.set_visible(!show_text_size);
            text_size_quick_spin.set_visible(show_text_size);
            size_icon.set_visible(!show_text_size);
            text_size_icon.set_visible(show_text_size);
            text_editor.refresh();
        }
    });

    {
        let state = state.clone();
        close_after_copy_switch.connect_active_notify(move |switch| {
            state.borrow_mut().close_after_copy = switch.is_active();
            save_settings(&state);
        });
    }
    {
        let settings_popover = settings_popover.clone();
        settings_button.connect_clicked(move |_| {
            settings_popover.popup();
        });
    }
    let dock_divider1 = gtk::Separator::new(gtk::Orientation::Vertical);
    dock_divider1.add_css_class("dock-divider");
    let dock_divider2 = gtk::Separator::new(gtk::Orientation::Vertical);
    dock_divider2.add_css_class("dock-divider");
    let dock_divider3 = gtk::Separator::new(gtk::Orientation::Vertical);
    dock_divider3.add_css_class("dock-divider");
    control_dock.append(&color_palette);
    control_dock.append(&dock_divider1);
    control_dock.append(&size_group);
    control_dock.append(&dock_divider2);
    control_dock.append(&zoom_group);
    control_dock.append(&zoom_actions);
    control_dock.append(&dock_divider3);
    control_dock.append(&settings_button);

    let bottom_rail = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::End)
        .margin_bottom(16)
        .build();
    let bottom_spacer_start = gtk::Box::builder().hexpand(true).build();
    let bottom_spacer_end = gtk::Box::builder().hexpand(true).build();
    bottom_rail.append(&bottom_spacer_start);
    bottom_rail.append(&control_dock);
    bottom_rail.append(&bottom_spacer_end);

    let editor_grid = gtk::Grid::builder().hexpand(true).vexpand(true).build();
    editor_grid.attach(&palette, 0, 0, 1, 1);
    editor_grid.attach(&editor_view, 1, 0, 1, 1);
    editor_grid.attach(&bottom_rail, 1, 1, 1, 1);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.add_css_class("editor-content");
    content.append(&editor_grid);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&content));
    overlay.add_overlay(&empty_state);

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
    text_editor.connect_window(&window);

    let zoom_updating = Rc::new(Cell::new(false));
    let fit_updating = Rc::new(Cell::new(false));

    let apply_background = {
        let drawing_area = drawing_area.clone();
        let scroller = scroller.clone();
        let empty_state = empty_state.clone();
        let palette = palette.clone();
        let control_dock = control_dock.clone();
        let state = state.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let fit_toggle = fit_toggle.clone();
        let zoom_updating = zoom_updating.clone();
        let fit_updating = fit_updating.clone();
        let text_editor = text_editor.clone();
        Rc::new(move |pixbuf: gdk_pixbuf::Pixbuf| {
            text_editor.finish(false);
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
            scroller.set_visible(true);
            empty_state.set_visible(false);
            palette.set_visible(true);
            control_dock.set_visible(true);
            scroller.hadjustment().set_value(0.0);
            scroller.vadjustment().set_value(0.0);
            drawing_area.queue_resize();
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
                set_source_file(Some(path));
                set_status("");
            }
            Err(err) => {
                let msg = format!("Failed to open image: {err}");
                set_status(&msg);
            }
        }
    }

    let (sender, receiver) = mpsc::channel::<CaptureResponse>();

    let set_status_for_timer = set_status.clone();
    let set_source_file_for_timer = set_source_file.clone();
    let button_for_timer = capture_button.clone();
    let apply_background_for_timer = apply_background.clone();
    let window_for_timer = window.clone();
    let app_for_timer = app.clone();
    let startup_capture = Rc::new(RefCell::new(startup_capture));
    if initial_capture {
        capture_button.set_sensitive(false);
    }

    glib::timeout_add_local(CAPTURE_RESPONSE_POLL_INTERVAL, move || {
        let startup_result = {
            let mut slot = startup_capture.borrow_mut();
            let received = slot
                .as_mut()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => Some(Err(CaptureFailure::failed(
                        "The capture task stopped before returning a screenshot.",
                    ))),
                    Err(mpsc::TryRecvError::Empty) => None,
                });
            if received.is_some() {
                slot.take();
            }
            received
        };

        let handle = |result: Result<String, CaptureFailure>, close_on_cancel: bool| -> bool {
            button_for_timer.set_sensitive(true);
            match result {
                Ok(uri) => {
                    let file = gio::File::for_uri(&uri);
                    match file.path() {
                        Some(path) => match gdk_pixbuf::Pixbuf::from_file(path) {
                            Ok(pixbuf) => {
                                apply_background_for_timer(pixbuf);
                                set_source_file_for_timer(file.path());
                                set_status_for_timer("");
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
                    present_window(&window_for_timer);
                    false
                }
                Err(err) if err.is_user_abort() && close_on_cancel => {
                    window_for_timer.set_visible(false);
                    app_for_timer.quit();
                    true
                }
                Err(err) if err.is_user_abort() => {
                    set_status_for_timer("");
                    present_window(&window_for_timer);
                    false
                }
                Err(err) => {
                    let msg = format!("Capture failed: {}", err.message);
                    set_status_for_timer(&msg);
                    present_window(&window_for_timer);
                    false
                }
            }
        };

        if let Some(result) = startup_result
            && handle(result, true)
        {
            return glib::ControlFlow::Break;
        }
        while let Ok(response) = receiver.try_recv() {
            if handle(response.result, response.close_on_cancel) {
                return glib::ControlFlow::Break;
            }
        }
        glib::ControlFlow::Continue
    });

    let start_capture = Rc::new({
        let runtime = runtime.clone();
        let set_status_for_capture = set_status.clone();
        let button = capture_button.clone();
        let window_for_capture = window.clone();
        move || {
            button.set_sensitive(false);
            set_status_for_capture("Capturing via portal...");

            let runtime = runtime.clone();
            let sender = sender.clone();
            let window_for_identifier = window_for_capture.clone();
            let window_for_hide = window_for_capture.clone();

            glib::MainContext::default().spawn_local(async move {
                let identifier = WindowIdentifier::from_native(&window_for_identifier).await;
                window_for_hide.set_visible(false);

                runtime.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let result = capture::capture_screenshot(identifier, CaptureMode::Auto).await;

                    let _ = sender.send(CaptureResponse {
                        result,
                        close_on_cancel: false,
                    });
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
    {
        let start_capture = start_capture.clone();
        empty_capture_button.connect_clicked(move |_| {
            start_capture();
        });
    }

    let state_for_draw = state.clone();
    let draw_area_for_draw = drawing_area.clone();
    let scroller_for_draw = scroller.clone();
    drawing_area.set_draw_func(move |_, ctx, width, height| {
        {
            let mut state = state_for_draw.borrow_mut();
            let scroller_width = scroller_for_draw.width();
            let scroller_height = scroller_for_draw.height();
            let viewport_width = if scroller_width > 1 {
                scroller_width
            } else {
                width
            }
            .max(1);
            let viewport_height = if scroller_height > 1 {
                scroller_height
            } else {
                height
            }
            .max(1);
            state.viewport_width = viewport_width;
            state.viewport_height = viewport_height;
            if let Some(background) = state.background.as_ref() {
                let (content_w, content_h) = if state.fit_to_window {
                    (1, 1)
                } else {
                    let (scale, _, _) = editor::view_transform(&state);
                    let scaled_w = (background.width() as f64 * scale).round() as i32;
                    let scaled_h = (background.height() as f64 * scale).round() as i32;
                    (
                        scaled_w.max(viewport_width).max(1),
                        scaled_h.max(viewport_height).max(1),
                    )
                };
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

    {
        let drawing_area = drawing_area.clone();
        let state = state.clone();
        let scroller = scroller.clone();
        let text_editor = text_editor.clone();
        drawing_area.connect_resize(move |area, _, _| {
            {
                let mut state = state.borrow_mut();
                state.viewport_width = scroller.width().max(1);
                state.viewport_height = scroller.height().max(1);
            }
            text_editor.refresh();
            area.queue_draw();
        });
    }

    let move_changed = Rc::new(Cell::new(false));
    let eraser_changed = Rc::new(Cell::new(false));
    let drag = gtk::GestureDrag::new();
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let move_changed = move_changed.clone();
        let eraser_changed = eraser_changed.clone();
        let sync_size_controls = sync_size_controls.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut state = state.borrow_mut();
            if state.editing_text().is_some() {
                state.drag_start_view = None;
                state.drag_last_image = None;
                return;
            }
            let Some(point) = editor::map_to_image_checked(&state, x, y) else {
                state.drag_start_view = None;
                state.drag_last_image = None;
                state.draft = None;
                return;
            };
            let point_view = Point { x, y };
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
            let sync_text = matches!(state.tool, Tool::Select);
            drop(state);
            if sync_text {
                sync_size_controls();
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
                    if let Some(index) = state.selected
                        && let Some(last) = state.drag_last_image
                    {
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
        let text_editor = text_editor.clone();
        let sync_size_controls = sync_size_controls.clone();
        click.connect_pressed(move |_, _, x, y| {
            if text_editor.active() {
                text_editor.finish(false);
                drawing_area.grab_focus();
                return;
            }
            let pos = {
                let state_ref = state.borrow();
                let Some(pos) = editor::map_to_image_checked(&state_ref, x, y) else {
                    if matches!(state_ref.tool, Tool::Select) {
                        drop(state_ref);
                        state.borrow_mut().selected = None;
                        sync_size_controls();
                        drawing_area.queue_draw();
                    }
                    return;
                };
                pos
            };
            let tool = state.borrow().tool;
            match tool {
                Tool::Text => {
                    let existing = {
                        let state = state.borrow();
                        editor::hit_test(&state.annotations, pos).filter(|&index| {
                            matches!(state.annotations[index], Annotation::Text { .. })
                        })
                    };
                    text_editor.begin(pos, existing);
                    sync_size_controls();
                }
                Tool::Select => {
                    let mut editor_state = state.borrow_mut();
                    editor_state.selected = editor::hit_test(&editor_state.annotations, pos);
                    drop(editor_state);
                    drawing_area.grab_focus();
                    sync_size_controls();
                    drawing_area.queue_draw();
                }
                _ => {}
            }
        });
    }
    {
        let state = state.clone();
        let move_changed = move_changed.clone();
        let text_editor = text_editor.clone();
        click.connect_released(move |_, _, x, y| {
            let existing = {
                let state = state.borrow();
                if state.tool != Tool::Select || move_changed.get() || text_editor.active() {
                    return;
                }
                let point = editor::map_to_image_checked(&state, x, y);
                state.selected.filter(|&index| {
                    matches!(state.annotations.get(index), Some(Annotation::Text { .. }))
                        && point.and_then(|point| editor::hit_test(&state.annotations, point))
                            == Some(index)
                })
            };
            if let Some(index) = existing {
                let pos = editor::map_to_image(&state.borrow(), x, y);
                text_editor.begin(pos, Some(index));
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
            let sync_size_controls = sync_size_controls.clone();
            let text_editor = text_editor.clone();
            let drawing_area = drawing_area.clone();
            button.connect_toggled(move |active_button| {
                if !active_button.is_active() {
                    return;
                }
                for (_, other) in buttons.iter() {
                    if other != active_button {
                        other.set_active(false);
                    }
                }
                text_editor.finish(false);
                let mut state = state.borrow_mut();
                state.tool = tool;
                state.draft = None;
                state.selected = None;
                drop(state);
                drawing_area.set_cursor_from_name((tool == Tool::Text).then_some("text"));
                drawing_area.queue_draw();
                sync_size_controls();
            });
        }

        for (tool, button) in buttons.iter() {
            if *tool == Tool::Pen {
                button.set_active(true);
                break;
            }
        }
    }

    let apply_color = Rc::new({
        let state = state.clone();
        let text_editor = text_editor.clone();
        move |color: gdk::RGBA| {
            {
                let mut state = state.borrow_mut();
                state.color = color;
                if let Some(index) = state.selected
                    && matches!(state.annotations.get(index), Some(Annotation::Text { color: current, .. }) if *current != color)
                {
                    if state.editing_text() != Some(index) {
                        state.record_change();
                    }
                    if let Some(Annotation::Text { color: current, .. }) =
                        state.annotations.get_mut(index)
                    {
                        *current = color;
                    }
                }
            }
            save_settings(&state);
            text_editor.refresh();
        }
    });

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
            let apply_color = apply_color.clone();
            button.connect_clicked(|button| button.set_active(true));
            button.connect_toggled(move |active_button| {
                if !active_button.is_active() {
                    return;
                }
                for (_, other) in buttons.iter() {
                    if other != active_button {
                        other.set_active(false);
                    }
                }
                apply_color(color);
            });
        }

        let current_color = state.borrow().color;
        for (color, button) in buttons.iter() {
            if (color.red() - current_color.red()).abs() < 0.01
                && (color.green() - current_color.green()).abs() < 0.01
                && (color.blue() - current_color.blue()).abs() < 0.01
            {
                button.set_active(true);
                break;
            }
        }
    }
    {
        let state = state.clone();
        let window = window.clone();
        let dialog = color_dialog.clone();
        let apply_color = apply_color.clone();
        color_button.connect_clicked(move |_| {
            let current = state.borrow().color;
            dialog.choose_rgba(Some(&window), Some(&current), None::<&gio::Cancellable>, {
                let apply_color = apply_color.clone();
                move |result| {
                    if let Ok(color) = result {
                        apply_color(color);
                    }
                }
            });
        });
    }
    {
        let state = state.clone();
        size_spin.connect_value_changed(move |spin| {
            state.borrow_mut().stroke_width = spin.value();
            save_settings(&state);
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let undo_action = gio::SimpleAction::new("undo", None);
        {
            let state = state.clone();
            let drawing_area = drawing_area.clone();
            let sync_size_controls = sync_size_controls.clone();
            let text_editor = text_editor.clone();
            undo_action.connect_activate(move |_, _| {
                if text_editor.handle_shortcut(gdk::Key::z, gdk::ModifierType::CONTROL_MASK) {
                    return;
                }
                state.borrow_mut().undo();
                sync_size_controls();
                drawing_area.queue_draw();
            });
        }
        window.add_action(&undo_action);
        app.set_accels_for_action("win.undo", &["<Control>z"]);

        let redo_action = gio::SimpleAction::new("redo", None);
        {
            let state = state.clone();
            let drawing_area = drawing_area.clone();
            let sync_size_controls = sync_size_controls.clone();
            let text_editor = text_editor.clone();
            redo_action.connect_activate(move |_, _| {
                if text_editor.handle_shortcut(gdk::Key::y, gdk::ModifierType::CONTROL_MASK) {
                    return;
                }
                state.borrow_mut().redo();
                sync_size_controls();
                drawing_area.queue_draw();
            });
        }
        window.add_action(&redo_action);
        app.set_accels_for_action("win.redo", &["<Control>y"]);

        {
            let state = state.clone();
            let drawing_area = drawing_area.clone();
            let sync_size_controls = sync_size_controls.clone();
            undo_button.connect_clicked(move |_| {
                state.borrow_mut().undo();
                sync_size_controls();
                drawing_area.queue_draw();
            });
        }
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let sync_size_controls = sync_size_controls.clone();
        redo_button.connect_clicked(move |_| {
            state.borrow_mut().redo();
            sync_size_controls();
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let sync_size_controls = sync_size_controls.clone();
        let key_controller = gtk::EventControllerKey::new();
        let text_editor = text_editor.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if text_editor.has_focus() {
                return glib::Propagation::Proceed;
            }
            if key != gdk::Key::Delete && key != gdk::Key::BackSpace {
                return glib::Propagation::Proceed;
            }

            let mut state = state.borrow_mut();
            if let Some(index) = state.selected.take()
                && state.remove_annotation(index).is_some()
            {
                drop(state);
                sync_size_controls();
                drawing_area.queue_draw();
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);
    }

    {
        let render_state = state.clone();
        let copy_behavior_state = state.clone();
        let set_status = set_status.clone();
        let set_status_for_render = set_status.clone();
        let text_editor_for_render = text_editor.clone();
        let render_for_copy = Rc::new(move || {
            text_editor_for_render.finish(false);
            let render_state = render_state.borrow();
            let Some(pixbuf) = editor::render_to_pixbuf(&render_state) else {
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
            let copy_behavior_state = copy_behavior_state.clone();
            let set_status = set_status.clone();
            move || {
                let Some(pixbuf) = render_for_copy() else {
                    return false;
                };
                if !copy_behavior_state.borrow().close_after_copy {
                    copy_to_gtk_clipboard(&pixbuf);
                    return false;
                }
                match output::copy_png_with_wl_copy(&pixbuf) {
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
            let text_editor = text_editor.clone();
            copy_action.connect_activate(move |_, _| {
                if text_editor.handle_shortcut(gdk::Key::c, gdk::ModifierType::CONTROL_MASK) {
                    return;
                }
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
        let set_source_file = set_source_file.clone();
        let apply_background = apply_background.clone();
        let file_dialog = gtk::FileDialog::new();
        file_dialog.set_title("Open Image");
        for button in [open_button.clone(), empty_open_button.clone()] {
            let window = window.clone();
            let set_status = set_status.clone();
            let set_source_file = set_source_file.clone();
            let apply_background = apply_background.clone();
            let file_dialog = file_dialog.clone();
            button.connect_clicked(move |_| {
                let apply_background = apply_background.clone();
                let set_status = set_status.clone();
                let set_source_file = set_source_file.clone();
                file_dialog.open(
                    Some(&window),
                    None::<&gio::Cancellable>,
                    move |res| match res {
                        Ok(file) => match file.path() {
                            Some(path) => match gdk_pixbuf::Pixbuf::from_file(&path) {
                                Ok(pixbuf) => {
                                    apply_background(pixbuf);
                                    set_source_file(Some(path));
                                    set_status("");
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
    }

    {
        let set_status = set_status.clone();
        let set_source_file = set_source_file.clone();
        let apply_background = apply_background.clone();
        for button in [paste_button.clone(), empty_paste_button.clone()] {
            let set_status = set_status.clone();
            let set_source_file = set_source_file.clone();
            let apply_background = apply_background.clone();
            button.connect_clicked(move |_| {
                if let Some(display) = gdk::Display::default() {
                    let clipboard = display.clipboard();
                    clipboard.read_texture_async(None::<&gio::Cancellable>, {
                        let set_status = set_status.clone();
                        let set_source_file = set_source_file.clone();
                        let apply_background = apply_background.clone();
                        move |res| match res {
                            Ok(Some(texture)) => {
                                #[allow(deprecated)]
                                if let Some(pixbuf) = gdk::pixbuf_get_from_texture(&texture) {
                                    apply_background(pixbuf);
                                    set_source_file(None);
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
    }

    {
        let window = window.clone();
        let state = state.clone();
        let set_status = set_status.clone();
        let file_dialog = gtk::FileDialog::new();
        let text_editor = text_editor.clone();
        save_button.connect_clicked(move |_| {
            text_editor.finish(false);
            let format = save_format_by_id(&state.borrow().save_format);
            let Some(pixbuf) = editor::render_to_pixbuf(&state.borrow()) else {
                set_status("Nothing to save yet.");
                return;
            };
            let initial_name = format!("wayshot.{}", format.extension);
            file_dialog.set_title(&format!("Save {}", format.label));
            file_dialog.set_initial_name(Some(&initial_name));
            let set_status = set_status.clone();
            file_dialog.save(
                Some(&window),
                None::<&gio::Cancellable>,
                move |res| match res {
                    Ok(file) => match file.path() {
                        Some(mut path) => {
                            path.set_extension(format.extension);
                            let result = if format.id == "jxl" {
                                jxl::save(&pixbuf, &path)
                            } else {
                                pixbuf
                                    .savev(&path, format.pixbuf_type, &[])
                                    .map_err(|error| error.to_string())
                            };
                            match result {
                                Ok(()) => set_status(&format!("Saved {}.", format.label)),
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
        let text_editor = text_editor.clone();
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
            drop(state);
            text_editor.refresh();
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let zoom_updating = zoom_updating.clone();
        let fit_updating = fit_updating.clone();
        let scroller = scroller.clone();
        let text_editor = text_editor.clone();
        fit_toggle.connect_toggled(move |toggle| {
            if fit_updating.get() {
                return;
            }
            let mut state = state.borrow_mut();
            state.fit_to_window = toggle.is_active();
            if state.fit_to_window {
                scroller.hadjustment().set_value(0.0);
                scroller.vadjustment().set_value(0.0);
                let (scale, _, _) = editor::view_transform(&state);
                zoom_updating.set(true);
                zoom_adjustment.set_value(scale);
                zoom_updating.set(false);
            }
            drop(state);
            text_editor.refresh();
            drawing_area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let drawing_area = drawing_area.clone();
        let zoom_adjustment = zoom_adjustment.clone();
        let zoom_updating = zoom_updating.clone();
        let text_editor = text_editor.clone();
        zoom_reset.connect_clicked(move |_| {
            let mut state = state.borrow_mut();
            state.fit_to_window = false;
            state.zoom = 1.0;
            zoom_updating.set(true);
            zoom_adjustment.set_value(1.0);
            zoom_updating.set(false);
            drop(state);
            text_editor.refresh();
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
        let text_editor = text_editor.clone();
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
            drop(state);
            text_editor.refresh();
            drawing_area_for_scroll.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(scroll);
    }
}

fn load_settings(state: &Rc<RefCell<EditorState>>) {
    let mut s = state.borrow_mut();
    let config_dir = glib::user_config_dir();
    let path = config_dir.join("wayshot").join("settings.ini");
    let kf = glib::KeyFile::new();

    // Set default color if file doesn't exist or loading fails
    s.color = gdk::RGBA::new(1.0, 0.30, 0.30, 1.0); // Red
    s.stroke_width = 4.0;
    s.text_size = 22.0;
    s.save_format = "png".to_string();
    s.close_after_copy = true;

    if kf.load_from_file(path, glib::KeyFileFlags::NONE).is_ok() {
        if let Ok(val) = kf.double("drawing", "stroke_width") {
            s.stroke_width = val;
        }
        if let Ok(val) = kf.double("drawing", "text_size") {
            s.text_size = val.clamp(8.0, 128.0);
        }
        if let Ok(val) = kf.string("output", "format") {
            s.save_format = val.to_string();
        }
        if let Ok(val) = kf.boolean("behavior", "close_after_copy") {
            s.close_after_copy = val;
        }
        if let Some(rgba) = kf
            .string("drawing", "color")
            .ok()
            .and_then(|val| gdk::RGBA::parse(&val).ok())
        {
            s.color = rgba;
        }
    }
}

fn save_settings(state: &Rc<RefCell<EditorState>>) {
    let s = state.borrow();
    let config_dir = glib::user_config_dir();
    let path_dir = config_dir.join("wayshot");
    let _ = std::fs::create_dir_all(&path_dir);
    let path = path_dir.join("settings.ini");

    let kf = glib::KeyFile::new();
    kf.set_double("drawing", "stroke_width", s.stroke_width);
    kf.set_double("drawing", "text_size", s.text_size);
    kf.set_string("drawing", "color", &s.color.to_string());
    kf.set_string("output", "format", &s.save_format);
    kf.set_boolean("behavior", "close_after_copy", s.close_after_copy);
    let _ = kf.save_to_file(path);
}
