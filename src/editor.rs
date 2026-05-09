use std::cell::RefCell;
use std::collections::HashMap;

use gdk_pixbuf::Pixbuf;
use gtk::cairo;
use gtk::gdk;
use gtk::gdk::prelude::GdkCairoContextExt;

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

pub struct EditorState {
    pub background: Option<Pixbuf>,
    pub annotations: Vec<Annotation>,
    pub history: Vec<Vec<Annotation>>,
    pub redo: Vec<Vec<Annotation>>,
    pub tool: Tool,
    pub color: gdk::RGBA,
    pub stroke_width: f64,
    pub text_size: f64,
    pub draft: Option<Annotation>,
    pub drag_start_view: Option<Point>,
    pub drag_last_image: Option<Point>,
    pub viewport_width: i32,
    pub viewport_height: i32,
    pub fit_to_window: bool,
    pub zoom: f64,
    pub selected: Option<usize>,
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
            draft: None,
            drag_start_view: None,
            drag_last_image: None,
            viewport_width: 0,
            viewport_height: 0,
            fit_to_window: true,
            zoom: 1.0,
            selected: None,
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
        self.pixelate_cache.borrow_mut().clear();
    }

    pub fn push_annotation(&mut self, annotation: Annotation) {
        self.record_change();
        self.annotations.push(annotation);
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.redo.push(self.annotations.clone());
            self.annotations = previous;
            self.draft = None;
            self.selected = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.history.push(self.annotations.clone());
            self.annotations = next;
            self.draft = None;
            self.selected = None;
        }
    }

    pub fn record_change(&mut self) {
        self.history.push(self.annotations.clone());
        self.redo.clear();
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
        ctx.set_source_pixbuf(bg, 0.0, 0.0);
        let _ = ctx.paint();
    }

    for annotation in state.annotations.iter() {
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

    if let Some(index) = state.selected {
        if let Some(bounds) = annotation_bounds(&state.annotations[index]) {
            let (x, y, w, h) = bounds.normalized();
            let _ = ctx.save();
            ctx.set_source_rgba(0.8, 0.8, 1.0, 0.6);
            ctx.set_line_width(1.0);
            ctx.rectangle(x, y, w, h);
            let _ = ctx.stroke();
            let _ = ctx.restore();
        }
    }
    let _ = ctx.restore();
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
    Point {
        x: ((x - offset_x) / scale).max(0.0),
        y: ((y - offset_y) / scale).max(0.0),
    }
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
        } => {
            let _ = ctx.save();
            set_source_rgba(ctx, color);
            ctx.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            ctx.set_font_size(*size);
            ctx.move_to(pos.x, pos.y);
            let _ = ctx.show_text(text);
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
            pos, text, size, ..
        } => {
            let width = (text.len() as f64 * size * 0.6).max(1.0);
            Some(Rect {
                x1: pos.x,
                y1: pos.y - size,
                x2: pos.x + width,
                y2: pos.y + size * 0.2,
            })
        }
        Annotation::Blur { rect, .. } => Some(*rect),
    }
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
    let _ = ctx.clip();
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
