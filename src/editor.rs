use std::cell::RefCell;
use std::collections::HashMap;

use gdk_pixbuf::Pixbuf;
use gtk::cairo;
use gtk::gdk;
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::pango;
use gtk::pango::prelude::*;

pub const MIN_TEXT_WIDTH: f64 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

impl Rect {
    pub fn normalized(self) -> (f64, f64, f64, f64) {
        let x = self.x1.min(self.x2);
        let y = self.y1.min(self.y2);
        let w = (self.x2 - self.x1).abs();
        let h = (self.y2 - self.y1).abs();
        (x, y, w, h)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Annotation {
    Pen {
        points: Vec<Point>,
        color: gdk::RGBA,
        width: f64,
    },
    Rect {
        rect: Rect,
        color: gdk::RGBA,
        width: f64,
    },
    Circle {
        rect: Rect,
        color: gdk::RGBA,
        width: f64,
    },
    Line {
        start: Point,
        end: Point,
        color: gdk::RGBA,
        width: f64,
        arrow: bool,
    },
    Text {
        pos: Point,
        text: String,
        color: gdk::RGBA,
        size: f64,
        width: f64,
    },
    Blur {
        rect: Rect,
        pixel_size: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Eraser,
    Rect,
    Circle,
    Line,
    Arrow,
    Text,
    Blur,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PixelateCacheKey {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pixel_size: i32,
}

struct TextEdit {
    index: usize,
    before: Vec<Annotation>,
}

pub struct EditorState {
    pub background: Option<Pixbuf>,
    pub annotations: Vec<Annotation>,
    pub history: Vec<Vec<Annotation>>,
    pub redo: Vec<Vec<Annotation>>,
    pub tool: Tool,
    pub color: gdk::RGBA,
    pub stroke_width: f64,
    pub text_size: f64,
    pub save_format: String,
    pub draft: Option<Annotation>,
    pub drag_start_view: Option<Point>,
    pub drag_last_image: Option<Point>,
    pub viewport_width: i32,
    pub viewport_height: i32,
    pub fit_to_window: bool,
    pub zoom: f64,
    pub close_after_copy: bool,
    pub selected: Option<usize>,
    text_edit: Option<TextEdit>,
    pixelate_cache: RefCell<HashMap<PixelateCacheKey, Pixbuf>>,
}

impl EditorState {
    pub fn new() -> Self {
        let color = gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
        Self {
            background: None,
            annotations: Vec::new(),
            history: Vec::new(),
            redo: Vec::new(),
            tool: Tool::Pen,
            color,
            stroke_width: 4.0,
            text_size: 22.0,
            save_format: "png".to_string(),
            draft: None,
            drag_start_view: None,
            drag_last_image: None,
            viewport_width: 0,
            viewport_height: 0,
            fit_to_window: true,
            zoom: 1.0,
            close_after_copy: true,
            selected: None,
            text_edit: None,
            pixelate_cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn set_background(&mut self, pixbuf: Pixbuf) {
        self.background = Some(pixbuf);
        self.annotations.clear();
        self.history.clear();
        self.redo.clear();
        self.draft = None;
        self.drag_start_view = None;
        self.drag_last_image = None;
        self.selected = None;
        self.text_edit = None;
        self.pixelate_cache.borrow_mut().clear();
    }

    pub fn push_annotation(&mut self, annotation: Annotation) {
        self.record_change();
        self.annotations.push(annotation);
    }

    pub fn undo(&mut self) {
        self.finish_text_edit(false);
        if let Some(previous) = self.history.pop() {
            self.redo.push(self.annotations.clone());
            self.annotations = previous;
            self.draft = None;
            self.selected = None;
        }
    }

    pub fn redo(&mut self) {
        self.finish_text_edit(false);
        if let Some(next) = self.redo.pop() {
            self.history.push(self.annotations.clone());
            self.annotations = next;
            self.draft = None;
            self.selected = None;
        }
    }

    pub fn record_change(&mut self) {
        self.finish_text_edit(false);
        self.history.push(self.annotations.clone());
        self.redo.clear();
    }

    pub fn editing_text(&self) -> Option<usize> {
        self.text_edit.as_ref().map(|edit| edit.index)
    }

    pub fn begin_text_edit(&mut self, index: usize) {
        self.finish_text_edit(false);
        if !matches!(self.annotations.get(index), Some(Annotation::Text { .. })) {
            return;
        }
        self.text_edit = Some(TextEdit {
            index,
            before: self.annotations.clone(),
        });
        self.selected = Some(index);
    }

    pub fn begin_new_text(&mut self, mut pos: Point) {
        self.finish_text_edit(false);
        let image_width = self
            .background
            .as_ref()
            .map_or(300.0, |bg| bg.width() as f64);
        let width = (image_width - pos.x)
            .min(300.0)
            .max(MIN_TEXT_WIDTH.min(image_width));
        pos.x = pos.x.min((image_width - width).max(0.0));
        if let Some(background) = self.background.as_ref() {
            let line_height = text_layout("", self.text_size, width).pixel_size().1 as f64;
            pos.y = pos
                .y
                .min((background.height() as f64 - line_height).max(0.0));
        }
        let index = self.annotations.len();
        self.text_edit = Some(TextEdit {
            index,
            before: self.annotations.clone(),
        });
        self.annotations.push(Annotation::Text {
            pos,
            text: String::new(),
            color: self.color,
            size: self.text_size,
            width,
        });
        self.selected = Some(index);
    }

    /// Keep a whole inline edit (typing, moving and resizing) as one undo step.
    pub fn finish_text_edit(&mut self, cancel: bool) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        if cancel {
            self.annotations = edit.before;
            self.selected = (edit.index < self.annotations.len()).then_some(edit.index);
            return;
        }
        if matches!(self.annotations.get(edit.index), Some(Annotation::Text { text, .. }) if text.trim().is_empty())
        {
            self.annotations.remove(edit.index);
            self.selected = None;
        }
        if self.annotations != edit.before {
            self.history.push(edit.before);
            self.redo.clear();
        }
    }

    pub fn discard_unchanged_record(&mut self) {
        if self.history.last() == Some(&self.annotations) {
            self.history.pop();
        }
    }

    pub fn remove_annotation(&mut self, index: usize) -> Option<Annotation> {
        if index >= self.annotations.len() {
            return None;
        }
        self.record_change();
        Some(self.annotations.remove(index))
    }

    pub fn erase_at(&mut self, point: Point) -> bool {
        if let Some(index) = hit_test(&self.annotations, point) {
            self.annotations.remove(index);
            self.selected = None;
            true
        } else {
            false
        }
    }
}

pub fn draw(state: &EditorState, ctx: &cairo::Context) {
    let (scale, offset_x, offset_y) = view_transform(state);
    let _ = ctx.save();
    ctx.translate(offset_x, offset_y);
    ctx.scale(scale, scale);

    if let Some(bg) = state.background.as_ref() {
        rounded_rect(
            ctx,
            0.0,
            0.0,
            bg.width() as f64,
            bg.height() as f64,
            18.0 / scale,
        );
        ctx.clip();
        ctx.set_source_pixbuf(bg, 0.0, 0.0);
        let _ = ctx.paint();
    }

    for (index, annotation) in state.annotations.iter().enumerate() {
        if state.editing_text() == Some(index) {
            continue;
        }
        draw_annotation(
            ctx,
            annotation,
            state.background.as_ref(),
            Some(&state.pixelate_cache),
        );
    }
    for annotation in state.draft.iter() {
        draw_annotation(ctx, annotation, state.background.as_ref(), None);
    }

    if let Some(index) = state.selected
        && state.editing_text() != Some(index)
        && let Some(bounds) = state.annotations.get(index).and_then(annotation_bounds)
    {
        let (x, y, w, h) = bounds.normalized();
        let _ = ctx.save();
        ctx.set_source_rgba(0.8, 0.8, 1.0, 0.6);
        ctx.set_line_width(1.0);
        ctx.rectangle(x, y, w, h);
        let _ = ctx.stroke();
        let _ = ctx.restore();
    }
    let _ = ctx.restore();
}

fn rounded_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let radius = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let right = x + w;
    let bottom = y + h;

    ctx.new_sub_path();
    ctx.arc(
        right - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    ctx.arc(
        right - radius,
        bottom - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    ctx.arc(
        x + radius,
        bottom - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    ctx.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        std::f64::consts::PI * 1.5,
    );
    ctx.close_path();
}

pub fn render_to_pixbuf(state: &EditorState) -> Option<Pixbuf> {
    let background = state.background.as_ref()?;
    let width = background.width();
    let height = background.height();
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;
    let ctx = cairo::Context::new(&surface).ok()?;
    ctx.set_source_pixbuf(background, 0.0, 0.0);
    let _ = ctx.paint();
    for annotation in state.annotations.iter() {
        draw_annotation(
            &ctx,
            annotation,
            Some(background),
            Some(&state.pixelate_cache),
        );
    }
    #[allow(deprecated)]
    gtk::gdk::pixbuf_get_from_surface(&surface, 0, 0, width, height)
}

pub fn view_transform(state: &EditorState) -> (f64, f64, f64) {
    let Some(background) = state.background.as_ref() else {
        return (1.0, 0.0, 0.0);
    };
    let img_w = background.width().max(1) as f64;
    let img_h = background.height().max(1) as f64;
    let vp_w = state.viewport_width.max(1) as f64;
    let vp_h = state.viewport_height.max(1) as f64;

    let scale = if state.fit_to_window {
        (vp_w / img_w).min(vp_h / img_h).max(0.01)
    } else {
        state.zoom.max(0.05)
    };
    let scaled_w = img_w * scale;
    let scaled_h = img_h * scale;
    let offset_x = ((vp_w - scaled_w) / 2.0).max(0.0);
    let offset_y = ((vp_h - scaled_h) / 2.0).max(0.0);
    (scale, offset_x, offset_y)
}

pub fn map_to_image(state: &EditorState, x: f64, y: f64) -> Point {
    let (scale, offset_x, offset_y) = view_transform(state);
    let (max_x, max_y) = state
        .background
        .as_ref()
        .map(|background| {
            (
                (background.width() - 1).max(0) as f64,
                (background.height() - 1).max(0) as f64,
            )
        })
        .unwrap_or((f64::MAX, f64::MAX));
    Point {
        x: ((x - offset_x) / scale).clamp(0.0, max_x),
        y: ((y - offset_y) / scale).clamp(0.0, max_y),
    }
}

pub fn map_to_image_checked(state: &EditorState, x: f64, y: f64) -> Option<Point> {
    let background = state.background.as_ref()?;
    let (scale, offset_x, offset_y) = view_transform(state);
    let img_w = background.width().max(1) as f64;
    let img_h = background.height().max(1) as f64;
    let image_x = (x - offset_x) / scale;
    let image_y = (y - offset_y) / scale;

    if image_x < 0.0 || image_y < 0.0 || image_x >= img_w || image_y >= img_h {
        return None;
    }

    Some(Point {
        x: image_x,
        y: image_y,
    })
}

fn draw_annotation(
    ctx: &cairo::Context,
    annotation: &Annotation,
    background: Option<&Pixbuf>,
    pixelate_cache: Option<&RefCell<HashMap<PixelateCacheKey, Pixbuf>>>,
) {
    match annotation {
        Annotation::Pen {
            points,
            color,
            width,
        } => {
            if points.len() < 2 {
                return;
            }
            let _ = ctx.save();
            set_source_rgba(ctx, color);
            ctx.set_line_width(*width);
            ctx.set_line_cap(cairo::LineCap::Round);
            ctx.set_line_join(cairo::LineJoin::Round);
            ctx.move_to(points[0].x, points[0].y);
            for point in points.iter().skip(1) {
                ctx.line_to(point.x, point.y);
            }
            let _ = ctx.stroke();
            let _ = ctx.restore();
        }
        Annotation::Rect { rect, color, width } => {
            let (x, y, w, h) = rect.normalized();
            let _ = ctx.save();
            set_source_rgba(ctx, color);
            ctx.set_line_width(*width);
            ctx.rectangle(x, y, w, h);
            let _ = ctx.stroke();
            let _ = ctx.restore();
        }
        Annotation::Circle { rect, color, width } => {
            let (x, y, w, h) = rect.normalized();
            if w < 1.0 || h < 1.0 {
                return;
            }
            let _ = ctx.save();
            set_source_rgba(ctx, color);
            ctx.set_line_width(*width);
            ellipse_path(ctx, x, y, w, h);
            let _ = ctx.stroke();
            let _ = ctx.restore();
        }
        Annotation::Line {
            start,
            end,
            color,
            width,
            arrow,
        } => {
            let _ = ctx.save();
            set_source_rgba(ctx, color);
            ctx.set_line_width(*width);
            ctx.set_line_cap(cairo::LineCap::Round);
            ctx.move_to(start.x, start.y);
            ctx.line_to(end.x, end.y);
            let _ = ctx.stroke();
            if *arrow {
                draw_arrow_head(ctx, *start, *end, *width, color);
            }
            let _ = ctx.restore();
        }
        Annotation::Text {
            pos,
            text,
            color,
            size,
            width,
        } => {
            let _ = ctx.save();
            set_source_rgba(ctx, color);
            ctx.move_to(pos.x, pos.y);
            pangocairo::functions::show_layout(ctx, &text_layout(text, *size, *width));
            let _ = ctx.restore();
        }
        Annotation::Blur { rect, pixel_size } => {
            if let Some(background) = background {
                draw_pixelate(ctx, *rect, *pixel_size, background, pixelate_cache);
            }
        }
    }
}

fn ellipse_path(ctx: &cairo::Context, x: f64, y: f64, width: f64, height: f64) {
    let kappa = 0.552_284_749_830_793_6;
    let rx = width / 2.0;
    let ry = height / 2.0;
    let cx = x + rx;
    let cy = y + ry;
    let ox = rx * kappa;
    let oy = ry * kappa;

    ctx.move_to(cx + rx, cy);
    ctx.curve_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry);
    ctx.curve_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy);
    ctx.curve_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry);
    ctx.curve_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy);
    ctx.close_path();
}

pub fn annotation_bounds(annotation: &Annotation) -> Option<Rect> {
    match annotation {
        Annotation::Pen { points, .. } => {
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for point in points {
                min_x = min_x.min(point.x);
                min_y = min_y.min(point.y);
                max_x = max_x.max(point.x);
                max_y = max_y.max(point.y);
            }
            if min_x.is_finite() {
                Some(Rect {
                    x1: min_x,
                    y1: min_y,
                    x2: max_x,
                    y2: max_y,
                })
            } else {
                None
            }
        }
        Annotation::Rect { rect, .. } => Some(*rect),
        Annotation::Circle { rect, .. } => Some(*rect),
        Annotation::Line { start, end, .. } => Some(Rect {
            x1: start.x,
            y1: start.y,
            x2: end.x,
            y2: end.y,
        }),
        Annotation::Text {
            pos,
            text,
            size,
            width,
            ..
        } => {
            let (_, height) = text_layout(text, *size, *width).pixel_size();
            Some(Rect {
                x1: pos.x,
                y1: pos.y,
                x2: pos.x + width,
                y2: pos.y + height as f64,
            })
        }
        Annotation::Blur { rect, .. } => Some(*rect),
    }
}

pub fn text_layout(text: &str, size: f64, width: f64) -> pango::Layout {
    let context = pangocairo::FontMap::default().create_context();
    pangocairo::functions::context_set_resolution(&context, 96.0);
    // Match GTK's fractional glyph positions, including at non-integer zoom.
    context.set_round_glyph_positions(false);
    let mut options = cairo::FontOptions::new().expect("Cairo font options");
    options.set_antialias(cairo::Antialias::Gray);
    options.set_hint_metrics(cairo::HintMetrics::Off);
    options.set_hint_style(cairo::HintStyle::Slight);
    pangocairo::functions::context_set_font_options(&context, Some(&options));
    let layout = pango::Layout::new(&context);
    let mut font = pango::FontDescription::from_string("Sans");
    font.set_absolute_size(size * pango::SCALE as f64);
    layout.set_font_description(Some(&font));
    layout.set_width((width.max(1.0) * pango::SCALE as f64).round() as i32);
    layout.set_wrap(pango::WrapMode::WordChar);
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrInt::new_insert_hyphens(false));
    layout.set_attributes(Some(&attributes));
    layout.set_text(text);
    layout
}

pub fn hit_test(annotations: &[Annotation], point: Point) -> Option<usize> {
    for (index, annotation) in annotations.iter().enumerate().rev() {
        if let Some(bounds) = annotation_bounds(annotation) {
            let (x, y, w, h) = bounds.normalized();
            if point.x >= x && point.x <= x + w && point.y >= y && point.y <= y + h {
                return Some(index);
            }
        }
    }
    None
}

pub fn move_annotation(annotation: &mut Annotation, dx: f64, dy: f64) {
    match annotation {
        Annotation::Pen { points, .. } => {
            for point in points.iter_mut() {
                point.x += dx;
                point.y += dy;
            }
        }
        Annotation::Rect { rect, .. } => {
            rect.x1 += dx;
            rect.y1 += dy;
            rect.x2 += dx;
            rect.y2 += dy;
        }
        Annotation::Circle { rect, .. } => {
            rect.x1 += dx;
            rect.y1 += dy;
            rect.x2 += dx;
            rect.y2 += dy;
        }
        Annotation::Line { start, end, .. } => {
            start.x += dx;
            start.y += dy;
            end.x += dx;
            end.y += dy;
        }
        Annotation::Text { pos, .. } => {
            pos.x += dx;
            pos.y += dy;
        }
        Annotation::Blur { rect, .. } => {
            rect.x1 += dx;
            rect.y1 += dy;
            rect.x2 += dx;
            rect.y2 += dy;
        }
    }
}

fn draw_arrow_head(ctx: &cairo::Context, start: Point, end: Point, width: f64, color: &gdk::RGBA) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.1 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let arrow_len = (10.0 * width).max(10.0);
    let arrow_width = (6.0 * width).max(6.0);
    let base_x = end.x - ux * arrow_len;
    let base_y = end.y - uy * arrow_len;
    let left_x = base_x + (-uy * arrow_width / 2.0);
    let left_y = base_y + (ux * arrow_width / 2.0);
    let right_x = base_x + (uy * arrow_width / 2.0);
    let right_y = base_y + (-ux * arrow_width / 2.0);

    let _ = ctx.save();
    set_source_rgba(ctx, color);
    ctx.move_to(end.x, end.y);
    ctx.line_to(left_x, left_y);
    ctx.line_to(right_x, right_y);
    ctx.close_path();
    let _ = ctx.fill();
    let _ = ctx.restore();
}

fn set_source_rgba(ctx: &cairo::Context, color: &gdk::RGBA) {
    ctx.set_source_rgba(
        color.red() as f64,
        color.green() as f64,
        color.blue() as f64,
        color.alpha() as f64,
    );
}

fn draw_pixelate(
    ctx: &cairo::Context,
    rect: Rect,
    pixel_size: i32,
    background: &Pixbuf,
    cache: Option<&RefCell<HashMap<PixelateCacheKey, Pixbuf>>>,
) {
    let Some((key, x, y, w, h)) = pixelate_geometry(rect, pixel_size, background) else {
        return;
    };

    let pixelated = if let Some(cache) = cache {
        if let Some(pixelated) = cache.borrow().get(&key).cloned() {
            pixelated
        } else {
            let Some(pixelated) = create_pixelated_pixbuf(background, key) else {
                return;
            };
            let mut cache = cache.borrow_mut();
            if cache.len() > 64 {
                cache.clear();
            }
            cache.insert(key, pixelated.clone());
            pixelated
        }
    } else {
        let Some(pixelated) = create_pixelated_pixbuf(background, key) else {
            return;
        };
        pixelated
    };

    let _ = ctx.save();
    ctx.rectangle(x, y, w, h);
    ctx.clip();
    ctx.set_source_pixbuf(&pixelated, x, y);
    let _ = ctx.paint();
    let _ = ctx.restore();
}

fn pixelate_geometry(
    rect: Rect,
    pixel_size: i32,
    background: &Pixbuf,
) -> Option<(PixelateCacheKey, f64, f64, f64, f64)> {
    let (x, y, w, h) = rect.normalized();
    if w < 1.0 || h < 1.0 {
        return None;
    }

    let max_w = background.width() as f64;
    let max_h = background.height() as f64;
    let x = x.max(0.0).min(max_w).round() as i32;
    let y = y.max(0.0).min(max_h).round() as i32;
    let w = w.min(max_w - x as f64).max(1.0).round() as i32;
    let h = h.min(max_h - y as f64).max(1.0).round() as i32;

    if x >= background.width() || y >= background.height() || w <= 0 || h <= 0 {
        return None;
    }

    let w = w.min(background.width() - x).max(1);
    let h = h.min(background.height() - y).max(1);
    let key = PixelateCacheKey {
        x,
        y,
        width: w,
        height: h,
        pixel_size: pixel_size.max(1),
    };

    Some((key, x as f64, y as f64, w as f64, h as f64))
}

fn create_pixelated_pixbuf(background: &Pixbuf, key: PixelateCacheKey) -> Option<Pixbuf> {
    let sub = Pixbuf::new_subpixbuf(background, key.x, key.y, key.width, key.height);

    let small_w = (key.width as f64 / key.pixel_size as f64).max(1.0).round() as i32;
    let small_h = (key.height as f64 / key.pixel_size as f64).max(1.0).round() as i32;
    let small = sub.scale_simple(small_w, small_h, gdk_pixbuf::InterpType::Nearest)?;
    small.scale_simple(key.width, key.height, gdk_pixbuf::InterpType::Nearest)
}

#[cfg(test)]
mod text_tests {
    use super::*;

    fn set_text(state: &mut EditorState, value: &str) {
        let index = state.editing_text().unwrap();
        if let Annotation::Text { text, .. } = &mut state.annotations[index] {
            *text = value.to_string();
        }
    }

    #[test]
    fn multiline_wrapping_and_hit_testing_follow_pango_metrics() {
        let text =
            "Ação: olá, 世界\n\nA long paragraph that wraps automatically across several lines.";
        let mut annotation = Annotation::Text {
            pos: Point { x: 20.0, y: 30.0 },
            text: text.to_string(),
            color: gdk::RGBA::BLACK,
            size: 22.0,
            width: 300.0,
        };
        let wide = annotation_bounds(&annotation).unwrap();
        if let Annotation::Text { width, .. } = &mut annotation {
            *width = 100.0;
        }
        let narrow = annotation_bounds(&annotation).unwrap();
        assert!(narrow.y2 > wide.y2);
        assert_eq!(narrow.x2 - narrow.x1, 100.0);
        let layout = text_layout(text, 22.0, 100.0);
        assert!(layout.line_count() > 3);
        assert_eq!(narrow.y2 - narrow.y1, layout.pixel_size().1 as f64);
        assert_eq!(
            hit_test(
                &[annotation.clone()],
                Point {
                    x: 25.0,
                    y: narrow.y2 - 1.0
                }
            ),
            Some(0)
        );
        assert_eq!(
            hit_test(
                &[annotation],
                Point {
                    x: 25.0,
                    y: narrow.y2 + 1.0
                }
            ),
            None
        );
    }

    #[test]
    fn text_sessions_commit_as_one_undo_step_and_restore_on_cancel() {
        let mut state = EditorState::new();
        state.begin_new_text(Point { x: 10.0, y: 20.0 });
        set_text(&mut state, "First line\nSecond line");
        state.finish_text_edit(false);
        let original = state.annotations.clone();
        assert_eq!(state.history.len(), 1);

        state.begin_text_edit(0);
        set_text(&mut state, "Edited\nMore text");
        move_annotation(&mut state.annotations[0], 30.0, 40.0);
        if let Annotation::Text { width, size, .. } = &mut state.annotations[0] {
            *width = 120.0;
            *size = 30.0;
        }
        let edited = state.annotations.clone();
        state.undo();
        assert_eq!(state.annotations, original);
        assert_eq!(state.editing_text(), None);
        state.redo();
        assert_eq!(state.annotations, edited);
        state.begin_text_edit(0);
        set_text(&mut state, "Canceled");
        move_annotation(&mut state.annotations[0], 50.0, 20.0);
        state.finish_text_edit(true);
        assert_eq!(state.annotations, edited);
        assert_eq!(state.history.len(), 2);
        state.undo();
        assert_eq!(state.annotations, original);
        state.undo();
        assert!(state.annotations.is_empty());
    }

    #[test]
    fn empty_or_canceled_new_text_keeps_history_and_redo() {
        let mut state = EditorState::new();
        state.begin_new_text(Point { x: 10.0, y: 20.0 });
        set_text(&mut state, "Original");
        state.finish_text_edit(false);
        state.undo();
        state.begin_new_text(Point { x: 10.0, y: 20.0 });
        set_text(&mut state, "  \n  ");
        state.finish_text_edit(false);
        assert!(state.annotations.is_empty());
        assert!(state.history.is_empty());
        assert_eq!(state.redo.len(), 1);
        state.begin_new_text(Point { x: 10.0, y: 20.0 });
        set_text(&mut state, "Canceled");
        state.finish_text_edit(true);
        assert!(state.annotations.is_empty());
        assert_eq!(state.redo.len(), 1);
        state.redo();
        state.begin_text_edit(0);
        set_text(&mut state, "");
        state.finish_text_edit(false);
        assert!(state.annotations.is_empty());
        state.undo();
        assert!(
            matches!(&state.annotations[0], Annotation::Text { text, .. } if text == "Original")
        );
    }

    #[test]
    fn new_text_near_image_edge_keeps_the_first_line_visible() {
        let mut state = EditorState::new();
        state.set_background(Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 400, 300).unwrap());
        state.begin_new_text(Point { x: 399.0, y: 299.0 });
        let bounds = annotation_bounds(&state.annotations[0]).unwrap();
        assert!(bounds.x1 >= 0.0 && bounds.x2 <= 400.0);
        assert!(bounds.y1 >= 0.0 && bounds.y2 <= 300.0);
    }

    #[test]
    fn export_includes_live_multiline_text_without_editor_chrome() {
        let mut state = EditorState::new();
        let background = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 400, 300).unwrap();
        background.fill(0xffffffff);
        state.set_background(background);
        state.begin_new_text(Point { x: 20.0, y: 30.0 });
        set_text(&mut state, "First line\nSecond line\nAção 世界");
        let live = render_to_pixbuf(&state).unwrap();
        let bytes = live.read_pixel_bytes();
        let stride = live.rowstride() as usize;
        let channels = live.n_channels() as usize;
        let line_height = text_layout("First line", 22.0, 300.0).pixel_size().1 as usize;
        for row in 0..3 {
            let top = 30 + row * line_height;
            assert!(
                (top..top + line_height)
                    .any(|y| { (20..200).any(|x| bytes[y * stride + x * channels] < 128) }),
                "line {row} must be present in the exported image"
            );
        }
        state.finish_text_edit(false);
        let committed = render_to_pixbuf(&state).unwrap();
        assert_eq!(live.read_pixel_bytes(), committed.read_pixel_bytes());
    }
}
