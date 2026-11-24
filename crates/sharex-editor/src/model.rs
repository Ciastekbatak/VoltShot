use crate::raster::render_document;
use serde::{Deserialize, Serialize};
use sharex_core::{CapturedImage, CoreError, Region};
use std::collections::HashSet;

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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

    fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
            && self.right().is_finite()
            && self.bottom().is_finite()
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
    SpeechBalloon,
    Magnifier,
    Spotlight,
    Crop,
}

impl EditorTool {
    pub const ALL: [Self; 16] = [
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
            | AnnotationKind::Spotlight { rect, .. } => *rect,
            AnnotationKind::SpeechBalloon { rect, tail, .. } => RectF::from_points(
                Point::new(rect.x.min(tail.x), rect.y.min(tail.y)),
                Point::new(rect.right().max(tail.x), rect.bottom().max(tail.y)),
            ),
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
            | AnnotationKind::Spotlight { rect, .. } => {
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
            AnnotationKind::StepNumber { center, .. }
            | AnnotationKind::Magnifier { center, .. } => {
                center.x += dx;
                center.y += dy;
            }
        }
    }

    fn resize(&mut self, new_rect: RectF) -> bool {
        match &mut self.kind {
            AnnotationKind::Rectangle { rect, .. }
            | AnnotationKind::Ellipse { rect, .. }
            | AnnotationKind::Pixelate { rect, .. }
            | AnnotationKind::Blur { rect, .. }
            | AnnotationKind::Spotlight { rect, .. } => {
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
                *tail = Point::new(
                    new_rect.x + scale_offset(tail.x - rect.x, rect.width, new_rect.width),
                    new_rect.y + scale_offset(tail.y - rect.y, rect.height, new_rect.height),
                );
                *rect = new_rect;
            }
            AnnotationKind::Magnifier { center, radius, .. } => {
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

    /// Resizes an annotation in one undoable edit. For a balloon, the rectangle is its body.
    pub fn resize_annotation(&mut self, id: u64, new_rect: RectF) -> bool {
        if !new_rect.is_valid() {
            return false;
        }
        let Some(index) = self.annotations.iter().position(|item| item.id == id) else {
            return false;
        };
        let mut resized = self.annotations[index].clone();
        if !resized.resize(new_rect) {
            return false;
        }
        self.begin_move_undo();
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
        assert!(source.resize_annotation(balloon, resized));
        assert!(!source.resize_annotation(balloon, resized));
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
            assert!(doc.resize_annotation(id, new_rect));
            assert_eq!(doc.annotations().last().unwrap().bounds(), new_rect);
        }
        let line = doc.push_annotation(AnnotationKind::Line {
            start: Point::new(1.0, 2.0),
            end: Point::new(5.0, 6.0),
            color: RgbaColor::RED,
            stroke_width: 1.0,
        });
        assert!(!doc.resize_annotation(line, new_rect));
        assert!(!doc.resize_annotation(999, new_rect));
        assert!(doc.undo());
        assert_eq!(doc.annotations().len(), 6);
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
