use crate::raster::render_document;
use sharex_core::{CapturedImage, CoreError, Region};

/// RGBA color value used by annotations and editor controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbaColor {
    pub const RED: Self = Self::rgb(235, 64, 52);
    pub const ORANGE: Self = Self::rgb(245, 145, 32);
    pub const YELLOW: Self = Self::rgb(250, 214, 48);
    pub const GREEN: Self = Self::rgb(52, 199, 89);
    pub const CYAN: Self = Self::rgb(46, 196, 232);
    pub const BLUE: Self = Self::rgb(59, 130, 246);
    pub const PURPLE: Self = Self::rgb(168, 85, 247);
    pub const WHITE: Self = Self::rgb(245, 247, 250);
    pub const BLACK: Self = Self::rgb(20, 22, 26);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

/// ShareX editor swatch presets.
pub const SHAREX_PALETTE: [RgbaColor; 9] = [
    RgbaColor::RED,
    RgbaColor::ORANGE,
    RgbaColor::YELLOW,
    RgbaColor::GREEN,
    RgbaColor::CYAN,
    RgbaColor::BLUE,
    RgbaColor::PURPLE,
    RgbaColor::WHITE,
    RgbaColor::BLACK,
];

/// 2D image-space coordinate in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Normalized image-space rectangle (`width >= 0`, `height >= 0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RectF {
    pub fn from_points(a: Point, b: Point) -> Self {
        let min_x = a.x.min(b.x);
        let min_y = a.y.min(b.y);
        let max_x = a.x.max(b.x);
        let max_y = a.y.max(b.y);
        Self {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn contains(self, pt: Point, pad: f32) -> bool {
        pt.x >= self.x - pad
            && pt.x <= self.right() + pad
            && pt.y >= self.y - pad
            && pt.y <= self.bottom() + pad
    }

    pub fn translate(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }
}

/// Active tool in the ShareX image editor toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorTool {
    Select,
    Rectangle,
    FilledRectangle,
    Ellipse,
    Arrow,
    Line,
    Freehand,
    Highlighter,
    Text,
    StepNumber,
    Pixelate,
    Blur,
    Crop,
}

impl EditorTool {
    pub const ALL: [Self; 13] = [
        Self::Select,
        Self::Rectangle,
        Self::FilledRectangle,
        Self::Ellipse,
        Self::Arrow,
        Self::Line,
        Self::Freehand,
        Self::Highlighter,
        Self::Text,
        Self::StepNumber,
        Self::Pixelate,
        Self::Blur,
        Self::Crop,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Rectangle => "Rect",
            Self::FilledRectangle => "Fill",
            Self::Ellipse => "Ellipse",
            Self::Arrow => "Arrow",
            Self::Line => "Line",
            Self::Freehand => "Pen",
            Self::Highlighter => "Highlight",
            Self::Text => "Text",
            Self::StepNumber => "Step",
            Self::Pixelate => "Pixelate",
            Self::Blur => "Blur",
            Self::Crop => "Crop",
        }
    }

    pub const fn shortcut_hint(self) -> &'static str {
        match self {
            Self::Select => "V",
            Self::Rectangle => "R",
            Self::FilledRectangle => "F",
            Self::Ellipse => "E",
            Self::Arrow => "A",
            Self::Line => "L",
            Self::Freehand => "D",
            Self::Highlighter => "H",
            Self::Text => "T",
            Self::StepNumber => "N",
            Self::Pixelate => "P",
            Self::Blur => "B",
            Self::Crop => "C",
        }
    }
}

/// Strongly typed annotation payload stored in an `EditorDocument`.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationKind {
    Rectangle {
        rect: RectF,
        color: RgbaColor,
        stroke_width: f32,
        filled: bool,
    },
    Ellipse {
        rect: RectF,
        color: RgbaColor,
        stroke_width: f32,
    },
    Line {
        start: Point,
        end: Point,
        color: RgbaColor,
        stroke_width: f32,
    },
    Arrow {
        start: Point,
        end: Point,
        color: RgbaColor,
        stroke_width: f32,
    },
    Freehand {
        points: Vec<Point>,
        color: RgbaColor,
        stroke_width: f32,
    },
    Highlighter {
        points: Vec<Point>,
        color: RgbaColor,
        stroke_width: f32,
    },
    Text {
        position: Point,
        text: String,
        color: RgbaColor,
        font_size: f32,
        background: Option<RgbaColor>,
    },
    StepNumber {
        center: Point,
        number: u32,
        color: RgbaColor,
        radius: f32,
    },
    Pixelate {
        rect: RectF,
        block_size: u32,
    },
    Blur {
        rect: RectF,
        radius: u32,
    },
}

/// Identified annotation layer inside an `EditorDocument`.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub id: u64,
    pub kind: AnnotationKind,
}

impl Annotation {
    pub fn bounds(&self) -> RectF {
        match &self.kind {
            AnnotationKind::Rectangle { rect, .. }
            | AnnotationKind::Ellipse { rect, .. }
            | AnnotationKind::Pixelate { rect, .. }
            | AnnotationKind::Blur { rect, .. } => *rect,
            AnnotationKind::Line { start, end, .. } | AnnotationKind::Arrow { start, end, .. } => {
                RectF::from_points(*start, *end)
            }
            AnnotationKind::Freehand { points, .. }
            | AnnotationKind::Highlighter { points, .. } => {
                if points.is_empty() {
                    return RectF {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    };
                }
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for p in points {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                RectF {
                    x: min_x,
                    y: min_y,
                    width: max_x - min_x,
                    height: max_y - min_y,
                }
            }
            AnnotationKind::Text {
                position,
                text,
                font_size,
                ..
            } => {
                let char_w = (*font_size * 0.65).max(6.0);
                let width = (text.chars().count().max(1) as f32) * char_w + 12.0;
                let height = *font_size + 10.0;
                RectF {
                    x: position.x,
                    y: position.y,
                    width,
                    height,
                }
            }
            AnnotationKind::StepNumber { center, radius, .. } => RectF {
                x: center.x - *radius,
                y: center.y - *radius,
                width: *radius * 2.0,
                height: *radius * 2.0,
            },
        }
    }

    pub fn hit_test(&self, pt: Point, tolerance: f32) -> bool {
        self.bounds().contains(pt, tolerance)
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        match &mut self.kind {
            AnnotationKind::Rectangle { rect, .. }
            | AnnotationKind::Ellipse { rect, .. }
            | AnnotationKind::Pixelate { rect, .. }
            | AnnotationKind::Blur { rect, .. } => {
                *rect = rect.translate(dx, dy);
            }
            AnnotationKind::Line { start, end, .. } | AnnotationKind::Arrow { start, end, .. } => {
                start.x += dx;
                start.y += dy;
                end.x += dx;
                end.y += dy;
            }
            AnnotationKind::Freehand { points, .. }
            | AnnotationKind::Highlighter { points, .. } => {
                for p in points {
                    p.x += dx;
                    p.y += dy;
                }
            }
            AnnotationKind::Text { position, .. } => {
                position.x += dx;
                position.y += dy;
            }
            AnnotationKind::StepNumber { center, .. } => {
                center.x += dx;
                center.y += dy;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct DocumentSnapshot {
    base_image: CapturedImage,
    annotations: Vec<Annotation>,
    next_step_number: u32,
}

/// Non-destructive screenshot editor document with full undo/redo history.
#[derive(Debug, Clone)]
pub struct EditorDocument {
    base_image: CapturedImage,
    annotations: Vec<Annotation>,
    selected_id: Option<u64>,
    next_id: u64,
    next_step_number: u32,
    undo_stack: Vec<DocumentSnapshot>,
    redo_stack: Vec<DocumentSnapshot>,
    dirty: bool,
}

impl EditorDocument {
    pub fn new(base_image: CapturedImage) -> Self {
        Self {
            base_image,
            annotations: Vec::new(),
            selected_id: None,
            next_id: 1,
            next_step_number: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
        }
    }

    pub fn base_image(&self) -> &CapturedImage {
        &self.base_image
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn selected_id(&self) -> Option<u64> {
        self.selected_id
    }

    pub fn select(&mut self, id: Option<u64>) {
        self.selected_id = id;
    }

    pub fn select_at(&mut self, pt: Point, tolerance: f32) -> Option<u64> {
        let hit = self
            .annotations
            .iter()
            .rev()
            .find(|a| a.hit_test(pt, tolerance))
            .map(|a| a.id);
        self.selected_id = hit;
        hit
    }

    pub fn next_step_number(&self) -> u32 {
        self.next_step_number
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            base_image: self.base_image.clone(),
            annotations: self.annotations.clone(),
            next_step_number: self.next_step_number,
        }
    }

    fn record_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
        self.redo_stack.clear();
        self.dirty = true;
    }

    pub fn push_annotation(&mut self, kind: AnnotationKind) -> u64 {
        self.record_undo();
        if matches!(kind, AnnotationKind::StepNumber { .. }) {
            self.next_step_number = self.next_step_number.saturating_add(1);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.annotations.push(Annotation { id, kind });
        self.selected_id = Some(id);
        id
    }

    pub fn begin_move_undo(&mut self) {
        self.record_undo();
    }

    pub fn translate_annotation(&mut self, id: u64, dx: f32, dy: f32) -> bool {
        if let Some(item) = self.annotations.iter_mut().find(|a| a.id == id) {
            item.translate(dx, dy);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn delete_selected(&mut self) -> bool {
        let Some(target) = self.selected_id else {
            return false;
        };
        if let Some(pos) = self.annotations.iter().position(|a| a.id == target) {
            self.record_undo();
            self.annotations.remove(pos);
            self.selected_id = None;
            true
        } else {
            false
        }
    }

    pub fn clear_annotations(&mut self) {
        if self.annotations.is_empty() {
            return;
        }
        self.record_undo();
        self.annotations.clear();
        self.selected_id = None;
        self.next_step_number = 1;
    }

    /// Crops the document base image and translates existing annotations into the cropped frame.
    pub fn crop(&mut self, region: Region) -> Result<(), CoreError> {
        let cropped = self.base_image.crop(region)?;
        self.record_undo();
        self.base_image = cropped;
        let dx = -(region.x() as f32);
        let dy = -(region.y() as f32);
        for item in &mut self.annotations {
            item.translate(dx, dy);
        }
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.snapshot());
        self.base_image = prev.base_image;
        self.annotations = prev.annotations;
        self.next_step_number = prev.next_step_number;
        self.selected_id = None;
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.snapshot());
        self.base_image = next.base_image;
        self.annotations = next.annotations;
        self.next_step_number = next.next_step_number;
        self.selected_id = None;
        self.dirty = true;
        true
    }

    /// Flattens the base image and all annotations into a single RGBA image.
    pub fn flatten(&self) -> Result<CapturedImage, CoreError> {
        render_document(self)
    }
}
