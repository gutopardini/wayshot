use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::editor::{self, Annotation, EditorState, Point};

const GRIP: i32 = 8;
// GtkTextView reserves one logical pixel for its insertion cursor.
const CURSOR_SPACE: i32 = 1;

/// A native text view over the canvas; its frame only receives events inside
/// the selected text box, leaving the rest of the drawing area interactive.
pub struct InlineTextEditor {
    frame: gtk::Grid,
    view: gtk::TextView,
    style_tag: gtk::TextTag,
    css: gtk::CssProvider,
    style: RefCell<String>,
    state: Rc<RefCell<EditorState>>,
    overlay: glib::WeakRef<gtk::Overlay>,
    canvas: glib::WeakRef<gtk::DrawingArea>,
    updating: Cell<bool>,
}

impl InlineTextEditor {
    pub fn new(
        state: Rc<RefCell<EditorState>>,
        overlay: &gtk::Overlay,
        canvas: &gtk::DrawingArea,
    ) -> Rc<Self> {
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        view.buffer().set_enable_undo(true);
        let style_tag = gtk::TextTag::builder().insert_hyphens(false).build();
        view.buffer().tag_table().add(&style_tag);
        view.update_property(&[gtk::accessible::Property::Label("Annotation text")]);

        let frame = gtk::Grid::builder().visible(false).build();
        frame.set_widget_name("wayshot-inline-text");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for grip in [&top, &bottom] {
            grip.set_height_request(GRIP);
            grip.set_cursor_from_name(Some("move"));
            grip.set_tooltip_text(Some("Drag to move text"));
        }
        for grip in [&left, &right] {
            grip.set_width_request(GRIP);
            grip.set_cursor_from_name(Some("ew-resize"));
            grip.set_tooltip_text(Some("Drag to resize text box"));
            grip.add_css_class("text-resize-grip");
        }
        frame.attach(&top, 0, 0, 3, 1);
        frame.attach(&left, 0, 1, 1, 1);
        frame.attach(&view, 1, 1, 1, 1);
        frame.attach(&right, 2, 1, 1, 1);
        frame.attach(&bottom, 0, 2, 3, 1);
        overlay.add_overlay(&frame);
        overlay.set_measure_overlay(&frame, false);
        overlay.set_clip_overlay(&frame, true);

        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &view.display(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let this = Rc::new(Self {
            frame,
            view,
            style_tag,
            css,
            style: RefCell::new(String::new()),
            state,
            overlay: overlay.downgrade(),
            canvas: canvas.downgrade(),
            updating: Cell::new(false),
        });

        let weak = Rc::downgrade(&this);
        overlay.connect_get_child_position(move |_, child| {
            let this = weak.upgrade()?;
            if child != this.frame.upcast_ref::<gtk::Widget>() {
                return None;
            }
            let state = this.state.borrow();
            let index = state.editing_text()?;
            let annotation = state.annotations.get(index)?;
            let (x, y, width, height) = editor::annotation_bounds(annotation)?.normalized();
            let (scale, offset_x, offset_y) = editor::view_transform(&state);
            let content_width = (width * scale).round().max(1.0) as i32 + CURSOR_SPACE;
            let (_, natural_height, _, _) =
                this.view.measure(gtk::Orientation::Vertical, content_width);
            Some(gdk::Rectangle::new(
                (offset_x + x * scale).round() as i32 - GRIP,
                (offset_y + y * scale).round() as i32 - GRIP,
                content_width + GRIP * 2,
                ((height * scale).ceil() as i32).max(natural_height) + GRIP * 2,
            ))
        });

        let weak = Rc::downgrade(&this);
        this.view.buffer().connect_changed(move |buffer| {
            let Some(this) = weak.upgrade() else { return };
            if this.updating.get() {
                return;
            }
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            {
                let mut state = this.state.borrow_mut();
                if let Some(index) = state.editing_text()
                    && let Some(Annotation::Text { text: current, .. }) =
                        state.annotations.get_mut(index)
                {
                    *current = text.to_string();
                }
            }
            this.refresh();
        });

        for grip in [&top, &bottom] {
            Self::connect_drag(&this, grip, None);
        }
        Self::connect_drag(&this, &left, Some(true));
        Self::connect_drag(&this, &right, Some(false));
        this
    }

    pub fn active(&self) -> bool {
        self.state.borrow().editing_text().is_some()
    }

    pub fn has_focus(&self) -> bool {
        self.view.has_focus()
    }

    pub fn begin(&self, point: Point, existing: Option<usize>) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(index) = existing {
                state.begin_text_edit(index);
            } else {
                state.begin_new_text(point);
            }
        }
        let text = {
            let state = self.state.borrow();
            let Some(index) = state.editing_text() else {
                return;
            };
            let Annotation::Text { text, .. } = &state.annotations[index] else {
                return;
            };
            text.clone()
        };
        self.updating.set(true);
        let buffer = self.view.buffer();
        buffer.set_enable_undo(false);
        buffer.set_text(&text);
        buffer.set_enable_undo(true);
        buffer.place_cursor(&buffer.end_iter());
        self.updating.set(false);
        self.refresh();
        self.view.grab_focus();
    }

    pub fn finish(&self, cancel: bool) {
        self.state.borrow_mut().finish_text_edit(cancel);
        self.refresh();
    }

    pub fn refresh(&self) {
        let state = self.state.borrow();
        let annotation = state.editing_text().and_then(|i| state.annotations.get(i));
        let Some(Annotation::Text { color, size, .. }) = annotation else {
            self.frame.set_visible(false);
            if let Some(canvas) = self.canvas.upgrade() {
                canvas.queue_draw();
            }
            return;
        };
        let (scale, _, _) = editor::view_transform(&state);
        let mut font = gtk::pango::FontDescription::from_string("Sans");
        font.set_absolute_size(size * scale * gtk::pango::SCALE as f64);
        self.style_tag.set_font_desc(Some(&font));
        self.style_tag.set_foreground_rgba(Some(color));
        let buffer = self.view.buffer();
        buffer.apply_tag(&self.style_tag, &buffer.start_iter(), &buffer.end_iter());
        let style = format!(
            "#wayshot-inline-text {{ outline: 1px solid #3584e4; background: alpha(#3584e4, 0.08); }}
             #wayshot-inline-text textview, #wayshot-inline-text textview text {{
                 background: transparent; color: {}; caret-color: {}; padding: 0;
                 border: 0; margin: 0; font-family: Sans; font-size: {}px;
                 font-weight: normal; font-style: normal;
             }}
             #wayshot-inline-text .text-resize-grip {{
                 background-image: linear-gradient(#3584e4, #3584e4);
                 background-size: 6px 14px; background-position: center; background-repeat: no-repeat;
             }}",
            color, color, size * scale,
        );
        if *self.style.borrow() != style {
            self.css.load_from_string(&style);
            *self.style.borrow_mut() = style;
        }
        drop(state);
        self.frame.set_visible(true);
        if let Some(overlay) = self.overlay.upgrade() {
            overlay.queue_allocate();
        }
        if let Some(canvas) = self.canvas.upgrade() {
            canvas.queue_draw();
        }
    }

    pub fn connect_window(self: &Rc<Self>, window: &adw::ApplicationWindow) {
        // Commit before toolbar actions (including copy/save) run. Canvas
        // clicks are handled there so the first outside click only commits.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        let weak_window = window.downgrade();
        click.connect_pressed(move |_, _, x, y| {
            let (Some(this), Some(window)) = (weak.upgrade(), weak_window.upgrade()) else {
                return;
            };
            if !this.active() {
                return;
            }
            let Some(target) = window.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            if &target == this.frame.upcast_ref::<gtk::Widget>() || target.is_ancestor(&this.frame)
            {
                return;
            }
            if let Some(canvas) = this.canvas.upgrade()
                && (&target == canvas.upcast_ref::<gtk::Widget>() || target.is_ancestor(&canvas))
            {
                return;
            }
            this.finish(false);
        });
        window.add_controller(click);

        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        key.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(this) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if this.handle_shortcut(key, modifiers) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(key);
    }

    // Also called by window actions in case the desktop dispatches an
    // accelerator before the key controller receives it.
    pub fn handle_shortcut(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        if !self.active() || !self.has_focus() {
            return false;
        }
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        if key == gdk::Key::Escape
            || (control && (key == gdk::Key::Return || key == gdk::Key::KP_Enter))
        {
            self.finish(key == gdk::Key::Escape);
            if let Some(canvas) = self.canvas.upgrade() {
                canvas.grab_focus();
            }
            return true;
        }
        if !control {
            return false;
        }
        let buffer = self.view.buffer();
        match key.to_lower() {
            gdk::Key::c => self.view.emit_copy_clipboard(),
            gdk::Key::z if !modifiers.contains(gdk::ModifierType::SHIFT_MASK) => {
                if buffer.can_undo() {
                    buffer.undo();
                }
            }
            gdk::Key::y | gdk::Key::z => {
                if buffer.can_redo() {
                    buffer.redo();
                }
            }
            _ => return false,
        }
        true
    }

    fn connect_drag(this: &Rc<Self>, grip: &gtk::Box, resize_left: Option<bool>) {
        let drag = gtk::GestureDrag::new();
        let origin = Rc::new(RefCell::new(None));
        let weak = Rc::downgrade(this);
        let start = origin.clone();
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(this) = weak.upgrade() else { return };
            let state = this.state.borrow();
            let pointer = gesture
                .current_event()
                .and_then(|event| event.position())
                .unwrap_or((x, y));
            *start.borrow_mut() = state
                .editing_text()
                .and_then(|i| state.annotations.get(i))
                .cloned()
                .map(|annotation| (annotation, pointer));
        });
        let weak = Rc::downgrade(this);
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some(this) = weak.upgrade() else { return };
            let Some((
                Annotation::Text {
                    pos: start,
                    width: start_width,
                    ..
                },
                pointer,
            )) = *origin.borrow()
            else {
                return;
            };
            // Surface coordinates stay stable when moving/reallocating the grip.
            let (dx, dy) = gesture
                .current_event()
                .and_then(|event| event.position())
                .map_or((dx, dy), |(x, y)| (x - pointer.0, y - pointer.1));
            {
                let mut state = this.state.borrow_mut();
                let (scale, _, _) = editor::view_transform(&state);
                let Some(index) = state.editing_text() else {
                    return;
                };
                let image_width = state
                    .background
                    .as_ref()
                    .map_or(f64::MAX, |bg| bg.width() as f64);
                let image_height = state
                    .background
                    .as_ref()
                    .map_or(f64::MAX, |bg| bg.height() as f64);
                let height = state
                    .annotations
                    .get(index)
                    .and_then(editor::annotation_bounds)
                    .map_or(0.0, |bounds| bounds.normalized().3);
                if let Some(Annotation::Text { pos, width, .. }) = state.annotations.get_mut(index)
                {
                    match resize_left {
                        Some(true) => {
                            let right = start.x + start_width;
                            pos.x = (start.x + dx / scale)
                                .clamp(0.0, (right - editor::MIN_TEXT_WIDTH).max(0.0));
                            *width = right - pos.x;
                        }
                        Some(false) => {
                            let available = (image_width - start.x).max(1.0);
                            *width = (start_width + dx / scale)
                                .clamp(editor::MIN_TEXT_WIDTH.min(available), available);
                        }
                        None => {
                            pos.x =
                                (start.x + dx / scale).clamp(0.0, (image_width - *width).max(0.0));
                            pos.y =
                                (start.y + dy / scale).clamp(0.0, (image_height - height).max(0.0));
                        }
                    }
                }
            }
            this.refresh();
        });
        grip.add_controller(drag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle() {
        let main_loop = glib::MainLoop::new(None, false);
        let done = main_loop.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || done.quit());
        main_loop.run();
    }

    fn drag_grip(frame: &gtk::Grid, column: i32, row: i32, dx: f64, dy: f64) {
        let grip = frame.child_at(column, row).unwrap();
        let drag = grip
            .observe_controllers()
            .item(0)
            .unwrap()
            .downcast::<gtk::GestureDrag>()
            .unwrap();
        drag.emit_by_name::<()>("drag-begin", &[&0.0_f64, &0.0_f64]);
        drag.emit_by_name::<()>("drag-update", &[&dx, &dy]);
        drag.emit_by_name::<()>("drag-end", &[&dx, &dy]);
        settle();
    }

    #[test]
    #[ignore = "requires a running GTK display; run with --ignored --test-threads=1"]
    fn inline_text_widget() {
        adw::init().expect("GTK display is required");
        let app = adw::Application::builder()
            .application_id("io.github.gutopardini.wayshot.TextTest")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let mut model = EditorState::new();
        let background =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 800, 600).unwrap();
        background.fill(0xffffffff);
        model.set_background(background);
        model.fit_to_window = false;
        model.viewport_width = 800;
        model.viewport_height = 600;
        let state = Rc::new(RefCell::new(model));
        let canvas = gtk::DrawingArea::builder()
            .content_width(800)
            .content_height(600)
            .focusable(true)
            .build();
        let draw_state = state.clone();
        canvas.set_draw_func(move |_, ctx, _, _| editor::draw(&draw_state.borrow(), ctx));
        let overlay = gtk::Overlay::builder().child(&canvas).build();
        let inline = InlineTextEditor::new(state.clone(), &overlay, &canvas);
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .content(&overlay)
            .default_width(800)
            .default_height(600)
            .build();
        inline.connect_window(&window);
        window.present();
        settle();
        inline.begin(Point { x: 100.0, y: 100.0 }, None);
        let text =
            "Ação: olá, 世界\n\nThis is a paragraph that wraps automatically over multiple lines.";
        inline.view.buffer().set_text(text);
        if let Annotation::Text { width, .. } = &mut state.borrow_mut().annotations[0] {
            *width = 240.0;
        }

        for (width, scale) in [
            (240.0, 1.0),
            (240.0, 0.65),
            (240.0, 2.0),
            (140.0, 0.65),
            (140.0, 2.0),
            (73.0, 0.5),
            (73.0, 1.75),
        ] {
            if let Annotation::Text { width: current, .. } = &mut state.borrow_mut().annotations[0]
            {
                *current = width;
            }
            state.borrow_mut().zoom = scale;
            inline.refresh();
            settle();
            overlay.allocate(800, 600, -1, None);
            assert!(inline.frame.is_visible());
            assert_eq!(
                inline.view.width(),
                (width * scale).round() as i32 + CURSOR_SPACE
            );
            let mut iter = inline.view.buffer().start_iter();
            let mut actual = vec![0];
            while inline.view.forward_display_line(&mut iter) {
                actual.push(iter.offset());
            }
            let expected: Vec<i32> = editor::text_layout(text, 22.0, width)
                .lines_readonly()
                .iter()
                .map(|line| text[..line.start_index() as usize].chars().count() as i32)
                .collect();
            assert_eq!(
                actual, expected,
                "wrapping must match export at width {width}, zoom {scale}"
            );
        }

        if let Annotation::Text { width, .. } = &mut state.borrow_mut().annotations[0] {
            *width = 240.0;
        }
        state.borrow_mut().zoom = 1.0;
        inline.refresh();
        settle();
        drag_grip(&inline.frame, 2, 1, -100.0, 0.0);
        let mut iter = inline.view.buffer().start_iter();
        let mut actual = vec![0];
        while inline.view.forward_display_line(&mut iter) {
            actual.push(iter.offset());
        }
        let expected: Vec<i32> = editor::text_layout(text, 22.0, 140.0)
            .lines_readonly()
            .iter()
            .map(|line| text[..line.start_index() as usize].chars().count() as i32)
            .collect();
        assert_eq!(actual, expected, "resized wrapping must match export");
        assert!(
            matches!(state.borrow().annotations[0], Annotation::Text { width, .. } if width == 140.0)
        );
        drag_grip(&inline.frame, 0, 0, 40.0, 20.0);
        assert!(
            matches!(state.borrow().annotations[0], Annotation::Text { pos, .. } if pos == Point { x: 140.0, y: 120.0 })
        );
        if let Some(dir) = std::env::var_os("WAYSHOT_TEST_ARTIFACT_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&overlay)).snapshot(
                &snapshot,
                overlay.width() as f64,
                overlay.height() as f64,
            );
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None).unwrap();
            let texture = renderer.render_texture(snapshot.to_node().unwrap(), None);
            texture.save_to_png(dir.join("inline-text.png")).unwrap();
            renderer.unrealize();
            editor::render_to_pixbuf(&state.borrow())
                .unwrap()
                .savev(dir.join("export.png"), "png", &[])
                .unwrap();
        }
        inline.finish(false);
        assert!(!inline.frame.is_visible());
        assert_eq!(state.borrow().history.len(), 1);
        let committed = state.borrow().annotations.clone();
        inline.begin(Point { x: 140.0, y: 120.0 }, Some(0));
        settle();
        let controllers = window.observe_controllers();
        let key = (0..controllers.n_items())
            .find_map(|i| {
                controllers
                    .item(i)?
                    .downcast::<gtk::EventControllerKey>()
                    .ok()
            })
            .unwrap();
        let buffer = inline.view.buffer();
        buffer.begin_user_action();
        buffer.insert_at_cursor(" Edited");
        buffer.end_user_action();
        assert!(key.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::z, &0_u32, &gdk::ModifierType::CONTROL_MASK]
        ));
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            text
        );
        assert!(key.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::y, &0_u32, &gdk::ModifierType::CONTROL_MASK]
        ));
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            format!("{text} Edited")
        );
        let copied = Rc::new(Cell::new(false));
        let observed_copy = copied.clone();
        inline.view.connect_copy_clipboard(move |view| {
            observed_copy.set(true);
            view.stop_signal_emission_by_name("copy-clipboard");
        });
        assert!(key.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::c, &0_u32, &gdk::ModifierType::CONTROL_MASK]
        ));
        assert!(copied.get());
        assert!(inline.active());
        assert!(key.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::Escape, &0_u32, &gdk::ModifierType::empty()]
        ));
        assert_eq!(state.borrow().annotations, committed);
        inline.begin(Point { x: 140.0, y: 120.0 }, Some(0));
        settle();
        buffer.set_text("Committed with Ctrl+Enter");
        assert!(key.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::Return, &0_u32, &gdk::ModifierType::CONTROL_MASK]
        ));
        assert!(!inline.active());
        assert!(
            matches!(&state.borrow().annotations[0], Annotation::Text { text, .. } if text == "Committed with Ctrl+Enter")
        );
        window.close();
    }
}
