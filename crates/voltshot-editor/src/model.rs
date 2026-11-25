use crate::raster::render_document;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use voltshot_core::{CapturedImage, CoreError, Region};

/// RGBA color value used by annotations and editor controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// VoltShot editor swatch presets.
pub const VOLTSHOT_PALETTE: [RgbaColor; 9] = [
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(crate) fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Normalized image-space rectangle (`width >= 0`, `height >= 0`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

    pub(crate) fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
            && self.right().is_finite()
            && self.bottom().is_finite()
    }
}

/// Active tool in the VoltShot image editor toolbar.
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
    SpeechBalloon,
    Magnifier,
    Spotlight,
    ImageOverlay,
    SmartEraser,
    Crop,
}

impl EditorTool {
    pub const ALL: [Self; 18] = [
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
        Self::SpeechBalloon,
        Self::Magnifier,
        Self::Spotlight,
        Self::ImageOverlay,
        Self::SmartEraser,
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
            Self::SpeechBalloon => "Balloon",
            Self::Magnifier => "Magnifier",
            Self::Spotlight => "Spotlight",
            Self::ImageOverlay => "Image Overlay",
            Self::SmartEraser => "Smart Eraser",
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
            Self::SpeechBalloon => "Q",
            Self::Magnifier => "M",
            Self::Spotlight => "S",
            Self::ImageOverlay => "I",
            Self::SmartEraser => "X",
            Self::Crop => "C",
        }
    }
}

/// Strongly typed annotation payload stored in an `EditorDocument`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    SpeechBalloon {
        rect: RectF,
        tail: Point,
        text: String,
        fill: RgbaColor,
        border: RgbaColor,
        text_color: RgbaColor,
        font_size: f32,
    },
    Magnifier {
        center: Point,
        radius: f32,
        zoom: f32,
        border_color: RgbaColor,
    },
    Spotlight {
        rect: RectF,
        dim_alpha: u8,
    },
    ImageOverlay {
        rect: RectF,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        opacity: f32,
    },
    SmartEraser {
        points: Vec<Point>,
        stroke_width: f32,
        sample_color: Option<RgbaColor>,
    },
}

/// A corner of the annotation's bounding rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeHandle {
    fn is_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }

    fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }
}

/// Identified annotation layer inside an `EditorDocument`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
            | AnnotationKind::Blur { rect, .. }
            | AnnotationKind::Spotlight { rect, .. }
            | AnnotationKind::ImageOverlay { rect, .. } => *rect,
            AnnotationKind::SpeechBalloon { rect, tail, .. } => RectF::from_points(
                Point::new(rect.x.min(tail.x), rect.y.min(tail.y)),
                Point::new(rect.right().max(tail.x), rect.bottom().max(tail.y)),
            ),
            AnnotationKind::Line { start, end, .. } | AnnotationKind::Arrow { start, end, .. } => {
                RectF::from_points(*start, *end)
            }
            AnnotationKind::Freehand { points, .. }
            | AnnotationKind::Highlighter { points, .. }
            | AnnotationKind::SmartEraser { points, .. } => {
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
            AnnotationKind::StepNumber { center, radius, .. }
            | AnnotationKind::Magnifier { center, radius, .. } => RectF {
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
            | AnnotationKind::Blur { rect, .. }
            | AnnotationKind::Spotlight { rect, .. }
            | AnnotationKind::ImageOverlay { rect, .. } => {
                *rect = rect.translate(dx, dy);
            }
            AnnotationKind::SpeechBalloon { rect, tail, .. } => {
                *rect = rect.translate(dx, dy);
                tail.x += dx;
                tail.y += dy;
            }
            AnnotationKind::Line { start, end, .. } | AnnotationKind::Arrow { start, end, .. } => {
                start.x += dx;
                start.y += dy;
                end.x += dx;
                end.y += dy;
            }
            AnnotationKind::Freehand { points, .. }
            | AnnotationKind::Highlighter { points, .. }
            | AnnotationKind::SmartEraser { points, .. } => {
                for p in points {
                    p.x += dx;
                    p.y += dy;
                }
            }
            AnnotationKind::Text { position, .. } => {
                position.x += dx;
                position.y += dy;
            }
            AnnotationKind::StepNumber { center, .. }
            | AnnotationKind::Magnifier { center, .. } => {
                center.x += dx;
                center.y += dy;
            }
        }
    }

    /// Returns the four corners of the annotation's bounds in image coordinates.
    pub fn resize_handles(&self) -> [(ResizeHandle, Point); 4] {
        let rect = self.bounds();
        [
            (ResizeHandle::TopLeft, Point::new(rect.x, rect.y)),
            (ResizeHandle::TopRight, Point::new(rect.right(), rect.y)),
            (ResizeHandle::BottomLeft, Point::new(rect.x, rect.bottom())),
            (
                ResizeHandle::BottomRight,
                Point::new(rect.right(), rect.bottom()),
            ),
        ]
    }

    fn supports_resize(&self) -> bool {
        !matches!(
            self.kind,
            AnnotationKind::Text { .. }
                | AnnotationKind::Freehand { .. }
                | AnnotationKind::Highlighter { .. }
                | AnnotationKind::SmartEraser { .. }
        )
    }

    fn resize_corner(&mut self, handle: ResizeHandle, target: Point) -> bool {
        let old = self.bounds();
        if !self.supports_resize() || !old.is_valid() || !target.is_finite() {
            return false;
        }

        let left = handle.is_left();
        let top = handle.is_top();
        let anchor = Point::new(
            if left { old.right() } else { old.x },
            if top { old.bottom() } else { old.y },
        );
        let corner = Point::new(
            if left { old.x } else { old.right() },
            if top { old.y } else { old.bottom() },
        );
        if target == corner {
            return false;
        }
        let horizontal = if left { -1.0 } else { 1.0 };
        let vertical = if top { -1.0 } else { 1.0 };

        if let AnnotationKind::Line { start, end, .. } | AnnotationKind::Arrow { start, end, .. } =
            &mut self.kind
        {
            if !start.is_finite() || !end.is_finite() {
                return false;
            }
            let start_moves = (start.x - corner.x).hypot(start.y - corner.y)
                <= (end.x - corner.x).hypot(end.y - corner.y);
            let resize_axis = |start: f32, end: f32, fixed: f32, target: f32, direction: f32| {
                if start == end {
                    return if start_moves {
                        (target, fixed)
                    } else {
                        (fixed, target)
                    };
                }
                let moving = fixed + direction * ((target - fixed) * direction).max(1.0);
                if (start < end) == (direction < 0.0) {
                    (moving, fixed)
                } else {
                    (fixed, moving)
                }
            };
            let (start_x, end_x) = resize_axis(start.x, end.x, anchor.x, target.x, horizontal);
            let (start_y, end_y) = resize_axis(start.y, end.y, anchor.y, target.y, vertical);
            let new_start = Point::new(start_x, start_y);
            let new_end = Point::new(end_x, end_y);
            let new = RectF::from_points(new_start, new_end);
            if !new.is_valid()
                || new.width.hypot(new.height) < 1.0
                || (*start, *end) == (new_start, new_end)
            {
                return false;
            }
            *start = new_start;
            *end = new_end;
            return true;
        }

        let dx = (target.x - anchor.x) * horizontal;
        let dy = (target.y - anchor.y) * vertical;
        let (width, height) = match &self.kind {
            AnnotationKind::Magnifier { .. } | AnnotationKind::StepNumber { .. } => {
                let side = (dx * 0.5 + dy * 0.5).max(1.0);
                (side, side)
            }
            AnnotationKind::SpeechBalloon { rect, .. } => (
                dx.max(old.width / rect.width.max(1.0)).max(1.0),
                dy.max(old.height / rect.height.max(1.0)).max(1.0),
            ),
            _ => (dx.max(1.0), dy.max(1.0)),
        };
        let moving = Point::new(anchor.x + horizontal * width, anchor.y + vertical * height);
        let new = RectF::from_points(anchor, moving);
        if !new.is_valid() || new.width < 1.0 || new.height < 1.0 || new == old {
            return false;
        }

        if let AnnotationKind::SpeechBalloon { rect, .. } = &self.kind {
            let scale_axis = |start: f32, length: f32, from: f32, span: f32, to: f32, size: f32| {
                if span > 0.0 {
                    (
                        to + (start - from) / span * size,
                        (length / span * size).max(1.0),
                    )
                } else {
                    (to, size)
                }
            };
            let (x, width) = scale_axis(rect.x, rect.width, old.x, old.width, new.x, new.width);
            let (y, height) = scale_axis(rect.y, rect.height, old.y, old.height, new.y, new.height);
            return self.resize_to_rect(RectF {
                x,
                y,
                width,
                height,
            });
        }
        self.resize_to_rect(new)
    }

    fn resize_to_rect(&mut self, new_rect: RectF) -> bool {
        if !new_rect.is_valid() || new_rect.width < 1.0 || new_rect.height < 1.0 {
            return false;
        }
        match &mut self.kind {
            AnnotationKind::Rectangle { rect, .. }
            | AnnotationKind::Ellipse { rect, .. }
            | AnnotationKind::Pixelate { rect, .. }
            | AnnotationKind::Blur { rect, .. }
            | AnnotationKind::Spotlight { rect, .. }
            | AnnotationKind::ImageOverlay { rect, .. } => {
                if *rect == new_rect {
                    return false;
                }
                *rect = new_rect;
            }
            AnnotationKind::SpeechBalloon { rect, tail, .. } => {
                if *rect == new_rect {
                    return false;
                }
                fn scale_offset(offset: f32, from: f32, to: f32) -> f32 {
                    if from > 0.0 {
                        offset / from * to
                    } else {
                        offset
                    }
                }
                let new_tail = Point::new(
                    new_rect.x + scale_offset(tail.x - rect.x, rect.width, new_rect.width),
                    new_rect.y + scale_offset(tail.y - rect.y, rect.height, new_rect.height),
                );
                if !new_tail.is_finite() {
                    return false;
                }
                *tail = new_tail;
                *rect = new_rect;
            }
            AnnotationKind::Magnifier { center, radius, .. }
            | AnnotationKind::StepNumber { center, radius, .. } => {
                let new_center = Point::new(
                    new_rect.x + new_rect.width * 0.5,
                    new_rect.y + new_rect.height * 0.5,
                );
                let new_radius = new_rect.width.min(new_rect.height) * 0.5;
                if *center == new_center && *radius == new_radius {
                    return false;
                }
                *center = new_center;
                *radius = new_radius;
            }
            _ => return false,
        }
        true
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
        while self.annotations.iter().any(|item| item.id == self.next_id) {
            self.next_id = self.next_id.wrapping_add(1).max(1);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
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

    /// Finds the nearest resize handle within the given image-space distance.
    pub fn hit_test_resize_handle(
        &self,
        id: u64,
        point: Point,
        tolerance: f32,
    ) -> Option<ResizeHandle> {
        if !point.is_finite() || !tolerance.is_finite() || tolerance < 0.0 {
            return None;
        }
        let annotation = self.annotations.iter().find(|item| item.id == id)?;
        if !annotation.supports_resize() || !annotation.bounds().is_valid() {
            return None;
        }
        annotation
            .resize_handles()
            .into_iter()
            .map(|(handle, corner)| (handle, (point.x - corner.x).hypot(point.y - corner.y)))
            .filter(|&(_, distance)| distance <= tolerance)
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(handle, _)| handle)
    }

    /// Drags a corner while anchoring its opposite corner, as one undoable edit.
    pub fn resize_annotation(
        &mut self,
        id: u64,
        handle: ResizeHandle,
        target_point: Point,
    ) -> bool {
        self.apply_resize(id, |annotation| {
            annotation.resize_corner(handle, target_point)
        })
    }

    /// Resizes a rectangular or circular annotation to explicit bounds in one undoable edit.
    /// For a balloon, the rectangle is its body.
    pub fn resize_annotation_to_rect(&mut self, id: u64, new_rect: RectF) -> bool {
        self.apply_resize(id, |annotation| annotation.resize_to_rect(new_rect))
    }

    fn apply_resize(&mut self, id: u64, resize: impl FnOnce(&mut Annotation) -> bool) -> bool {
        let Some(index) = self.annotations.iter().position(|item| item.id == id) else {
            return false;
        };
        let mut resized = self.annotations[index].clone();
        if !resize(&mut resized) || !resized.bounds().is_valid() {
            return false;
        }
        self.record_undo();
        self.annotations[index] = resized;
        true
    }

    /// Exports the ordered annotation layers as a JSON array for an .sxed project.
    pub fn export_annotations_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(&self.annotations).map_err(|err| {
            CoreError::ConfigError(format!("cannot export .sxed annotations: {err}"))
        })
    }

    /// Replaces the annotation layers in one undoable edit, using the current base image.
    pub fn import_annotations_json(&mut self, json: &str) -> Result<usize, CoreError> {
        let annotations: Vec<Annotation> = serde_json::from_str(json).map_err(|err| {
            CoreError::ConfigError(format!("cannot import .sxed annotations: {err}"))
        })?;
        let mut ids = HashSet::with_capacity(annotations.len());
        for item in &annotations {
            if item.id == 0 || !ids.insert(item.id) {
                return Err(CoreError::ConfigError(format!(
                    "invalid or duplicate .sxed annotation ID {}",
                    item.id
                )));
            }
        }
        let count = annotations.len();
        if annotations == self.annotations {
            return Ok(count);
        }
        let next_step_number = annotations
            .iter()
            .filter_map(|item| match item.kind {
                AnnotationKind::StepNumber { number, .. } => Some(number),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        if let Some(max_id) = ids.into_iter().max() {
            self.next_id = self.next_id.max(max_id.saturating_add(1));
        }
        self.record_undo();
        self.annotations = annotations;
        self.selected_id = None;
        self.next_step_number = next_step_number;
        Ok(count)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> EditorDocument {
        EditorDocument::new(CapturedImage::from_rgba(32, 32, vec![255; 32 * 32 * 4]).unwrap())
    }

    #[test]
    fn resize_and_project_json_round_trip_with_undo_and_redo() {
        let original = RectF::from_points(Point::new(3.0, 4.0), Point::new(13.0, 14.0));
        let resized = RectF::from_points(Point::new(6.0, 8.0), Point::new(26.0, 28.0));
        let mut source = document();
        let balloon = source.push_annotation(AnnotationKind::SpeechBalloon {
            rect: original,
            tail: Point::new(8.0, 18.0),
            text: "Note".into(),
            fill: RgbaColor::WHITE,
            border: RgbaColor::RED,
            text_color: RgbaColor::BLACK,
            font_size: 12.0,
        });
        source.push_annotation(AnnotationKind::Spotlight {
            rect: original,
            dim_alpha: 150,
        });
        source.push_annotation(AnnotationKind::Magnifier {
            center: Point::new(12.0, 15.0),
            radius: 4.0,
            zoom: 2.0,
            border_color: RgbaColor::GREEN,
        });
        source.push_annotation(AnnotationKind::StepNumber {
            center: Point::new(10.0, 15.0),
            number: 7,
            color: RgbaColor::BLUE,
            radius: 3.0,
        });
        let before = source.annotations().to_vec();
        assert!(source.resize_annotation_to_rect(balloon, resized));
        assert!(!source.resize_annotation_to_rect(balloon, resized));
        assert!(matches!(
            &source.annotations()[0].kind,
            AnnotationKind::SpeechBalloon { rect, tail, .. }
                if *rect == resized && *tail == Point::new(16.0, 36.0)
        ));
        assert!(source.undo());
        assert_eq!(source.annotations(), before);
        assert!(source.redo());

        let json = source.export_annotations_json().unwrap();
        let mut imported = document();
        imported.push_annotation(AnnotationKind::Blur {
            rect: original,
            radius: 2,
        });
        let prior = imported.annotations().to_vec();
        assert_eq!(imported.import_annotations_json(&json).unwrap(), 4);
        assert_eq!(imported.annotations(), source.annotations());
        assert_eq!(imported.next_step_number(), 8);
        assert_eq!(imported.export_annotations_json().unwrap(), json);
        assert!(imported.undo());
        assert_eq!(imported.annotations(), prior);
        assert_eq!(imported.next_step_number(), 1);
        assert!(imported.redo());
        assert_eq!(imported.annotations(), source.annotations());
        assert_eq!(imported.next_step_number(), 8);
        let new_id = imported.push_annotation(AnnotationKind::Pixelate {
            rect: original,
            block_size: 2,
        });
        assert!(source.annotations().iter().all(|item| item.id != new_id));
    }

    #[test]
    fn resize_updates_shape_and_effect_bounds_and_rejects_other_kinds() {
        let rect = RectF::from_points(Point::new(1.0, 2.0), Point::new(5.0, 6.0));
        let new_rect = rect.translate(4.0, 5.0);
        let mut doc = document();
        for kind in [
            AnnotationKind::Rectangle {
                rect,
                color: RgbaColor::RED,
                stroke_width: 1.0,
                filled: false,
            },
            AnnotationKind::Ellipse {
                rect,
                color: RgbaColor::RED,
                stroke_width: 1.0,
            },
            AnnotationKind::Pixelate {
                rect,
                block_size: 4,
            },
            AnnotationKind::Blur { rect, radius: 2 },
            AnnotationKind::Spotlight {
                rect,
                dim_alpha: 150,
            },
            AnnotationKind::Magnifier {
                center: Point::new(3.0, 4.0),
                radius: 2.0,
                zoom: 2.0,
                border_color: RgbaColor::GREEN,
            },
        ] {
            let id = doc.push_annotation(kind);
            assert!(doc.resize_annotation_to_rect(id, new_rect));
            assert_eq!(doc.annotations().last().unwrap().bounds(), new_rect);
        }
        let line = doc.push_annotation(AnnotationKind::Line {
            start: Point::new(1.0, 2.0),
            end: Point::new(5.0, 6.0),
            color: RgbaColor::RED,
            stroke_width: 1.0,
        });
        assert!(!doc.resize_annotation_to_rect(line, new_rect));
        assert!(!doc.resize_annotation_to_rect(999, new_rect));
        assert!(doc.undo());
        assert_eq!(doc.annotations().len(), 6);
    }

    #[test]
    fn corner_handles_resize_shapes_and_round_trip_through_history() {
        let upper_left = Point::new(4.0, 6.0);
        let bottom_right = Point::new(14.0, 16.0);
        let rect = RectF::from_points(upper_left, bottom_right);
        let target = Point::new(24.0, 26.0);
        let color = RgbaColor::RED;
        let kinds = [
            AnnotationKind::Rectangle {
                rect,
                color,
                stroke_width: 1.0,
                filled: false,
            },
            AnnotationKind::Ellipse {
                rect,
                color,
                stroke_width: 1.0,
            },
            AnnotationKind::SpeechBalloon {
                rect: RectF {
                    height: 7.0,
                    ..rect
                },
                tail: Point::new(9.0, 16.0),
                text: "Note".into(),
                fill: RgbaColor::WHITE,
                border: color,
                text_color: RgbaColor::BLACK,
                font_size: 12.0,
            },
            AnnotationKind::Magnifier {
                center: Point::new(9.0, 11.0),
                radius: 5.0,
                zoom: 2.0,
                border_color: color,
            },
            AnnotationKind::Spotlight {
                rect,
                dim_alpha: 128,
            },
            AnnotationKind::Arrow {
                start: bottom_right,
                end: upper_left,
                color,
                stroke_width: 1.0,
            },
            AnnotationKind::Line {
                start: upper_left,
                end: bottom_right,
                color,
                stroke_width: 1.0,
            },
            AnnotationKind::ImageOverlay {
                rect,
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 4],
                opacity: 1.0,
            },
        ];
        for kind in kinds {
            let mut doc = document();
            let id = doc.push_annotation(kind.clone());
            assert_eq!(
                doc.annotations()[0].resize_handles(),
                [
                    (ResizeHandle::TopLeft, upper_left),
                    (ResizeHandle::TopRight, Point::new(14.0, 6.0)),
                    (ResizeHandle::BottomLeft, Point::new(4.0, 16.0)),
                    (ResizeHandle::BottomRight, bottom_right),
                ]
            );
            for (handle, point) in doc.annotations()[0].resize_handles() {
                assert_eq!(doc.hit_test_resize_handle(id, point, 20.0), Some(handle));
            }
            assert_eq!(
                doc.hit_test_resize_handle(id, Point::new(9.0, 11.0), 2.0),
                None
            );
            assert_eq!(doc.hit_test_resize_handle(id + 1, bottom_right, 2.0), None);
            assert!(doc.resize_annotation(id, ResizeHandle::BottomRight, target));
            assert_eq!(
                doc.annotations()[0].bounds(),
                RectF::from_points(upper_left, target)
            );
            let resized = doc.annotations()[0].kind.clone();
            match &resized {
                AnnotationKind::Arrow { start, end, .. } => {
                    assert_eq!((*start, *end), (target, upper_left))
                }
                AnnotationKind::Line { start, end, .. } => {
                    assert_eq!((*start, *end), (upper_left, target))
                }
                AnnotationKind::Magnifier { center, radius, .. } => {
                    assert_eq!((*center, *radius), (Point::new(14.0, 16.0), 10.0))
                }
                AnnotationKind::SpeechBalloon { rect, tail, .. } => {
                    assert_eq!((rect.height, *tail), (14.0, Point::new(14.0, 26.0)))
                }
                _ => {}
            }
            assert!(!doc.resize_annotation(id, ResizeHandle::BottomRight, target));
            assert!(!doc.resize_annotation(
                id,
                ResizeHandle::BottomRight,
                Point::new(f32::NAN, 0.0)
            ));
            assert!(doc.undo());
            assert_eq!(doc.annotations()[0].kind, kind);
            assert!(doc.redo());
            assert_eq!(doc.annotations()[0].kind, resized);
        }
    }

    #[test]
    fn corner_resize_clamps_crossed_corners_and_handles_flat_lines() {
        let mut doc = document();
        let rect = RectF::from_points(Point::new(2.0, 3.0), Point::new(10.0, 11.0));
        let shape = doc.push_annotation(AnnotationKind::Ellipse {
            rect,
            color: RgbaColor::RED,
            stroke_width: 1.0,
        });
        assert!(doc.resize_annotation(shape, ResizeHandle::TopLeft, Point::new(25.0, 30.0)));
        assert_eq!(
            doc.annotations()[0].bounds(),
            RectF::from_points(Point::new(9.0, 10.0), Point::new(10.0, 11.0))
        );

        let line = doc.push_annotation(AnnotationKind::Line {
            start: Point::new(9.0, 5.0),
            end: Point::new(1.0, 5.0),
            color: RgbaColor::RED,
            stroke_width: 1.0,
        });
        let handle = doc
            .hit_test_resize_handle(line, Point::new(9.0, 5.0), 0.0)
            .unwrap();
        assert!(doc.resize_annotation(line, handle, Point::new(0.0, 5.0)));
        assert_eq!(
            doc.annotations()[1].bounds(),
            RectF::from_points(Point::new(1.0, 5.0), Point::new(2.0, 5.0))
        );
        assert!(doc.resize_annotation(line, handle, Point::new(5.0, 9.0)));
        assert!(
            matches!(doc.annotations()[1].kind, AnnotationKind::Line { start, end, .. }
            if start == Point::new(5.0, 9.0) && end == Point::new(1.0, 5.0))
        );
    }

    #[test]
    fn invalid_project_import_leaves_annotations_and_history_intact() {
        let mut doc = document();
        let rect = RectF::from_points(Point::new(0.0, 0.0), Point::new(5.0, 5.0));
        doc.push_annotation(AnnotationKind::Spotlight {
            rect,
            dim_alpha: 100,
        });
        let item = serde_json::to_string(&doc.annotations()[0]).unwrap();
        assert!(doc.undo());
        assert!(
            doc.import_annotations_json(&format!("[{item},{item}]"))
                .is_err()
        );
        assert!(doc.import_annotations_json("not JSON").is_err());
        assert!(doc.annotations().is_empty());
        assert!(doc.redo());
        assert_eq!(doc.annotations().len(), 1);
    }
}
