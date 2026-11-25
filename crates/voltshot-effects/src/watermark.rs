use rayon::prelude::*;
use voltshot_core::{CapturedImage, CoreError};

use crate::effects::{
    RgbaColor, WatermarkAnchor, blend_source_over, checked_rgba_len, unit_interval,
};

pub(crate) struct WatermarkPlacement {
    pub(crate) position: WatermarkAnchor,
    pub(crate) margin: u32,
    pub(crate) opacity: f32,
}

fn anchor_origin(canvas: i64, content: i64, margin: u32, align: std::cmp::Ordering) -> i64 {
    let room = canvas - content;
    let inset = i64::from(margin).min(room.max(0));
    match align {
        std::cmp::Ordering::Less => inset,
        std::cmp::Ordering::Equal => room.div_euclid(2),
        std::cmp::Ordering::Greater => room - inset,
    }
}

fn paint_overlay(
    image: &CapturedImage,
    width: i64,
    height: i64,
    placement: WatermarkPlacement,
    color_at: impl Fn(i64, i64) -> [u8; 4] + Sync,
) -> Result<CapturedImage, CoreError> {
    use WatermarkAnchor::*;
    use std::cmp::Ordering::{Equal, Greater, Less};

    let opacity = unit_interval(placement.opacity);
    if opacity == 0.0 || width == 0 || height == 0 {
        return Ok(image.clone());
    }

    let horizontal = match placement.position {
        TopLeft | MiddleLeft | BottomLeft => Less,
        TopCenter | Center | BottomCenter => Equal,
        TopRight | MiddleRight | BottomRight => Greater,
    };
    let vertical = match placement.position {
        TopLeft | TopCenter | TopRight => Less,
        MiddleLeft | Center | MiddleRight => Equal,
        BottomLeft | BottomCenter | BottomRight => Greater,
    };
    let canvas_width = i64::from(image.width());
    let canvas_height = i64::from(image.height());
    let origin_x = anchor_origin(canvas_width, width, placement.margin, horizontal);
    let origin_y = anchor_origin(canvas_height, height, placement.margin, vertical);
    let left = origin_x.max(0);
    let right = (origin_x + width).min(canvas_width);
    let top = origin_y.max(0);
    let bottom = (origin_y + height).min(canvas_height);
    let stride = canvas_width as usize * 4;
    let mut out = image.rgba().to_vec();
    out[top as usize * stride..bottom as usize * stride]
        .par_chunks_exact_mut(stride)
        .with_min_len(8)
        .enumerate()
        .for_each(|(row_index, row)| {
            let row_index = row_index as i64 + top - origin_y;
            for (offset, pixel) in row[left as usize * 4..right as usize * 4]
                .chunks_exact_mut(4)
                .enumerate()
            {
                let column = offset as i64 + left - origin_x;
                let mut color = color_at(column, row_index);
                color[3] = (f64::from(color[3]) * opacity).round() as u8;
                blend_source_over(pixel, color);
            }
        });
    CapturedImage::from_rgba(image.width(), image.height(), out)
}

pub(crate) fn apply_image_watermark(
    image: &CapturedImage,
    rgba: &[u8],
    width: u32,
    height: u32,
    placement: WatermarkPlacement,
) -> Result<CapturedImage, CoreError> {
    let expected = checked_rgba_len(width, height).unwrap_or(usize::MAX);
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(CoreError::InvalidImageBuffer {
            width,
            height,
            expected,
            actual: rgba.len(),
        });
    }
    paint_overlay(
        image,
        i64::from(width),
        i64::from(height),
        placement,
        |column, row_index| {
            let index = (row_index as usize * width as usize + column as usize) * 4;
            [
                rgba[index],
                rgba[index + 1],
                rgba[index + 2],
                rgba[index + 3],
            ]
        },
    )
}

pub(crate) fn apply_watermark_text(
    image: &CapturedImage,
    text: &str,
    color: RgbaColor,
    scale: u32,
    shadow: bool,
    placement: WatermarkPlacement,
) -> Result<CapturedImage, CoreError> {
    if unit_interval(placement.opacity) == 0.0 || color.a == 0 || text.is_empty() {
        return Ok(image.clone());
    }
    let lines: Vec<Vec<_>> = text
        .lines()
        .map(|line| line.chars().map(glyph).collect())
        .collect();
    let columns = lines.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return Ok(image.clone());
    }

    let scale = i64::from(scale.max(1));
    let shadow_span = i64::from(shadow);
    let extent = |count: usize, advance: i64| {
        (count as i64)
            .saturating_mul(advance)
            .saturating_sub(1 - shadow_span)
            .saturating_mul(scale)
    };
    let ink = |column: i64, row_index: i64| {
        if column < 0 || row_index < 0 || column % 6 == 5 || row_index % 8 == 7 {
            return false;
        }
        lines
            .get((row_index / 8) as usize)
            .and_then(|line| line.get((column / 6) as usize))
            .is_some_and(|glyph| glyph[(row_index % 8) as usize] & (16 >> (column % 6)) != 0)
    };
    let shadow_alpha = color.a.div_ceil(2);
    paint_overlay(
        image,
        extent(columns, 6),
        extent(lines.len(), 8),
        placement,
        |column, row_index| {
            let (column, row_index) = (column / scale, row_index / scale);
            if ink(column, row_index) {
                color.to_array()
            } else if shadow && ink(column - 1, row_index - 1) {
                [0, 0, 0, shadow_alpha]
            } else {
                [0; 4]
            }
        },
    )
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 17, 21, 27, 17],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' | '\t' => [0; 7],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '"' => [10, 10, 10, 0, 0, 0, 0],
        '#' => [10, 10, 31, 10, 31, 10, 10],
        '$' => [4, 15, 20, 14, 5, 30, 4],
        '%' => [24, 25, 2, 4, 8, 19, 3],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '*' => [0, 4, 21, 14, 21, 4, 0],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        ',' => [0, 0, 0, 0, 0, 4, 8],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0, 4],
        '/' => [1, 1, 2, 4, 8, 16, 16],
        ':' => [0, 4, 0, 0, 4, 0, 0],
        ';' => [0, 4, 0, 0, 4, 4, 8],
        '<' => [2, 4, 8, 16, 8, 4, 2],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '@' => [14, 17, 23, 21, 23, 16, 14],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        '\\' => [16, 16, 8, 4, 2, 1, 1],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '^' => [4, 10, 17, 0, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '`' => [8, 4, 0, 0, 0, 0, 0],
        '{' => [3, 4, 4, 8, 4, 4, 3],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        '}' => [24, 4, 4, 2, 4, 4, 24],
        '~' => [0, 0, 9, 22, 0, 0, 0],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}
