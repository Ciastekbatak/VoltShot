use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use voltshot_core::{CapturedImage, CoreError, Region};

use crate::resize::resize_lanczos;
use crate::watermark::{WatermarkPlacement, apply_image_watermark, apply_watermark_text};

/// Maximum width or height of an effect output in pixels.
pub const MAX_EFFECT_DIMENSION: u32 = 16_384;

/// Maximum pixel count of an effect output (256 MB for RGBA8).
pub const MAX_EFFECT_PIXELS: u64 = 64_000_000;

/// Checks whether the given dimensions fit within the effect allocation limits.
pub fn validate_output_dimensions(width: u32, height: u32) -> Result<(), CoreError> {
    if width > MAX_EFFECT_DIMENSION
        || height > MAX_EFFECT_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_EFFECT_PIXELS
    {
        return Err(CoreError::InvalidRegion(format!(
            "effect dimensions {width}x{height} exceed the limits of {MAX_EFFECT_DIMENSION} pixels per side and {MAX_EFFECT_PIXELS} total pixels"
        )));
    }
    Ok(())
}

/// RGBA8 color used by image effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl EffectColor {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Alternate name for an RGBA8 effect color.
pub type RgbaColor = EffectColor;

/// Alignment of a watermark relative to the captured image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// Composable image transformation applied to a [`CapturedImage`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageEffect {
    Border {
        width: u32,
        color: EffectColor,
    },
    CanvasPadding {
        top: u32,
        right: u32,
        bottom: u32,
        left: u32,
        color: EffectColor,
    },
    RoundedCorners {
        radius: u32,
    },
    /// Expands the canvas and composites a blurred source-alpha silhouette behind the image.
    /// Larger radii use three box passes per axis to approximate a Gaussian within that radius.
    DropShadow {
        offset_x: i32,
        offset_y: i32,
        blur_radius: u32,
        color: EffectColor,
        background: EffectColor,
    },
    Resize {
        width: u32,
        height: u32,
        keep_aspect: bool,
    },
    Grayscale,
    Invert,
    BrightnessContrast {
        brightness: i16,
        contrast: i16,
    },
    AutoCrop {
        tolerance: u8,
    },
    /// Draws scaled 5x7 ASCII bitmap text (lowercase renders as uppercase).
    /// An optional black shadow extends one cell down and right.
    WatermarkText {
        text: String,
        position: WatermarkAnchor,
        color: RgbaColor,
        opacity: f32,
        scale: u32,
        margin: u32,
        shadow: bool,
    },
    /// Blends the original RGB with its luminance multiplied by the tint RGB, preserving alpha.
    Colorize {
        color: RgbaColor,
        strength: f32,
    },
    /// Offsets deterministic horizontal bands in alternating directions; exposed pixels are transparent.
    Slice {
        min_height: u32,
        max_height: u32,
        shift: i32,
    },
    /// Composites a tightly packed RGBA8 stamp, clipping it to the destination if needed.
    ImageWatermark {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        position: WatermarkAnchor,
        opacity: f32,
        margin: u32,
    },
}

/// Applies a single [`ImageEffect`] to an image and returns the transformed copy.
pub fn apply_effect(
    image: &CapturedImage,
    effect: &ImageEffect,
) -> Result<CapturedImage, CoreError> {
    validate_output_dimensions(image.width(), image.height())?;
    match *effect {
        ImageEffect::Border { width, color } => {
            apply_canvas_padding(image, width, width, width, width, color)
        }
        ImageEffect::CanvasPadding {
            top,
            right,
            bottom,
            left,
            color,
        } => apply_canvas_padding(image, top, right, bottom, left, color),
        ImageEffect::RoundedCorners { radius } => apply_rounded_corners(image, radius),
        ImageEffect::DropShadow {
            offset_x,
            offset_y,
            blur_radius,
            color,
            background,
        } => apply_drop_shadow(image, offset_x, offset_y, blur_radius, color, background),
        ImageEffect::Resize {
            width,
            height,
            keep_aspect,
        } => apply_resize(image, width, height, keep_aspect),
        ImageEffect::Grayscale => apply_grayscale(image),
        ImageEffect::Invert => apply_invert(image),
        ImageEffect::BrightnessContrast {
            brightness,
            contrast,
        } => apply_brightness_contrast(image, brightness, contrast),
        ImageEffect::AutoCrop { tolerance } => apply_auto_crop(image, tolerance),
        ImageEffect::WatermarkText {
            ref text,
            position,
            color,
            opacity,
            scale,
            margin,
            shadow,
        } => apply_watermark_text(
            image,
            text,
            color,
            scale,
            shadow,
            WatermarkPlacement {
                position,
                margin,
                opacity,
            },
        ),
        ImageEffect::Colorize { color, strength } => apply_colorize(image, color, strength),
        ImageEffect::Slice {
            min_height,
            max_height,
            shift,
        } => apply_slice(image, min_height, max_height, shift),
        ImageEffect::ImageWatermark {
            ref rgba,
            width,
            height,
            position,
            opacity,
            margin,
        } => apply_image_watermark(
            image,
            rgba,
            width,
            height,
            WatermarkPlacement {
                position,
                margin,
                opacity,
            },
        ),
    }
}

/// Applies a sequence of [`ImageEffect`] transformations in order.
pub fn apply_effects(
    image: &CapturedImage,
    effects: &[ImageEffect],
) -> Result<CapturedImage, CoreError> {
    let Some((first, rest)) = effects.split_first() else {
        validate_output_dimensions(image.width(), image.height())?;
        return Ok(image.clone());
    };
    let mut current = apply_effect(image, first)?;
    for effect in rest {
        current = apply_effect(&current, effect)?;
    }
    Ok(current)
}

pub(crate) fn checked_rgba_len(width: u32, height: u32) -> Result<usize, CoreError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|&bytes| bytes <= isize::MAX as usize)
        .ok_or_else(|| {
            CoreError::InvalidRegion(format!(
                "image dimensions {width}x{height} overflow buffer allocation"
            ))
        })
}

fn apply_canvas_padding(
    image: &CapturedImage,
    top: u32,
    right: u32,
    bottom: u32,
    left: u32,
    color: EffectColor,
) -> Result<CapturedImage, CoreError> {
    if top == 0 && right == 0 && bottom == 0 && left == 0 {
        return Ok(image.clone());
    }

    let new_w = image
        .width()
        .checked_add(left)
        .and_then(|v| v.checked_add(right))
        .ok_or_else(|| CoreError::InvalidRegion("padded image width overflows u32".into()))?;
    let new_h = image
        .height()
        .checked_add(top)
        .and_then(|v| v.checked_add(bottom))
        .ok_or_else(|| CoreError::InvalidRegion("padded image height overflows u32".into()))?;

    validate_output_dimensions(new_w, new_h)?;
    let mut out = vec![0; checked_rgba_len(new_w, new_h)?];
    let src_stride = (image.width() as usize) * 4;
    let dst_stride = (new_w as usize) * 4;
    let left_bytes = (left as usize) * 4;
    let src = image.rgba();

    let fill = |bytes: &mut [u8]| {
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color.to_array());
        }
    };
    out.par_chunks_exact_mut(dst_stride)
        .with_min_len(8)
        .enumerate()
        .for_each(|(row_index, row)| {
            if let Some(source_y) = row_index
                .checked_sub(top as usize)
                .filter(|&row_index| row_index < image.height() as usize)
            {
                let (left, image_and_right) = row.split_at_mut(left_bytes);
                let (image_row, right) = image_and_right.split_at_mut(src_stride);
                fill(left);
                image_row.copy_from_slice(&src[source_y * src_stride..][..src_stride]);
                fill(right);
            } else {
                fill(row);
            }
        });

    CapturedImage::from_rgba(new_w, new_h, out)
}

fn apply_rounded_corners(image: &CapturedImage, radius: u32) -> Result<CapturedImage, CoreError> {
    if radius == 0 {
        return Ok(image.clone());
    }

    let width = image.width();
    let height = image.height();
    let wf = f64::from(width);
    let hf = f64::from(height);
    let corner_radius = f64::from(radius).min(wf / 2.0).min(hf / 2.0);
    let mut out = image.rgba().to_vec();

    let corner_pixels = corner_radius.ceil() as usize;
    out.par_chunks_exact_mut(width as usize * 4)
        .with_min_len(8)
        .enumerate()
        .for_each(|(row_index, row)| {
            let cy = row_index as f64 + 0.5;
            let corner_y = if cy < corner_radius {
                Some(corner_radius)
            } else if cy > hf - corner_radius {
                Some(hf - corner_radius)
            } else {
                None
            };

            let Some(ky) = corner_y else {
                return;
            };

            let left = 0..corner_pixels;
            let right = (width as usize - corner_pixels).max(corner_pixels)..width as usize;
            for column in left.chain(right) {
                let cx = column as f64 + 0.5;
                let corner_x = if cx < corner_radius {
                    Some(corner_radius)
                } else if cx > wf - corner_radius {
                    Some(wf - corner_radius)
                } else {
                    None
                };

                let Some(kx) = corner_x else {
                    continue;
                };

                let dist = (cx - kx).hypot(cy - ky);
                if dist <= corner_radius - 1.0 {
                    continue;
                }

                let alpha = &mut row[column * 4 + 3];
                if dist >= corner_radius {
                    *alpha = 0;
                } else {
                    let coverage = (corner_radius - dist).clamp(0.0, 1.0);
                    *alpha = (f64::from(*alpha) * coverage).round() as u8;
                }
            }
        });

    CapturedImage::from_rgba(width, height, out)
}

fn box_blur_horizontal(source: &[f32], out: &mut [f32], width: usize, radius: usize) {
    let weight = 1.0 / (2 * radius + 1) as f32;
    out.par_chunks_exact_mut(width)
        .zip(source.par_chunks_exact(width))
        .with_min_len(8)
        .for_each(|(out, row)| {
            let mut sum: f32 = row[..radius].iter().sum();
            for (column, output) in out.iter_mut().enumerate() {
                if column + radius < width {
                    sum += row[column + radius];
                }
                *output = sum * weight;
                if column >= radius {
                    sum -= row[column - radius];
                }
            }
        });
}

fn box_blur_vertical(source: &[f32], out: &mut [f32], width: usize, radius: usize) {
    let height = source.len() / width;
    let weight = 1.0 / (2 * radius + 1) as f32;
    let rows_per_band = (2 * radius + 1).max(32).min(height);
    out.par_chunks_mut(width * rows_per_band)
        .enumerate()
        .for_each(|(band, rows)| {
            let start = band * rows_per_band;
            let mut sums = vec![0.0; width];
            let zero_row = vec![0.0; width];
            let top = start.saturating_sub(radius);
            let bottom = (start + radius).min(height);
            for input in source[top * width..bottom * width].chunks_exact(width) {
                for (sum, &value) in sums.iter_mut().zip(input) {
                    *sum += value;
                }
            }
            let row_at = |row_index: usize| &source[row_index * width..][..width];
            for (index, row) in rows.chunks_exact_mut(width).enumerate() {
                let row_index = start + index;
                let incoming = if row_index + radius < height {
                    row_at(row_index + radius)
                } else {
                    &zero_row
                };
                let outgoing = if row_index >= radius {
                    row_at(row_index - radius)
                } else {
                    &zero_row
                };
                for (((sum, output), &add), &remove) in
                    sums.iter_mut().zip(row).zip(incoming).zip(outgoing)
                {
                    let current = *sum + add;
                    *output = current * weight;
                    *sum = current - remove;
                }
            }
        });
}

fn blur_alpha_channel(alpha: &mut [f32], width: usize, radius: usize) {
    if radius == 0 {
        return;
    }

    let mut scratch = vec![0.0; alpha.len()];
    if radius <= 2 {
        blur_small_radius(alpha, &mut scratch, width, radius);
        return;
    }
    for pass in 0..3 {
        let box_radius = (radius + pass) / 3;
        if box_radius != 0 {
            box_blur_horizontal(alpha, &mut scratch, width, box_radius);
            box_blur_vertical(&scratch, alpha, width, box_radius);
        }
    }
}

fn blur_small_radius(alpha: &mut [f32], scratch: &mut [f32], width: usize, radius: usize) {
    let height = alpha.len() / width;
    let sigma = (radius as f32 / 2.5).max(0.5);
    let denominator = 2.0 * sigma * sigma;
    let mut kernel = [0.0; 5];
    let weights = &mut kernel[..2 * radius + 1];
    for (index, weight) in weights.iter_mut().enumerate() {
        let distance = index as f32 - radius as f32;
        *weight = (-(distance * distance) / denominator).exp();
    }
    let sum: f32 = weights.iter().sum();
    for weight in &mut *weights {
        *weight /= sum;
    }

    let sample = |center: usize, length: usize, value_at: &dyn Fn(usize) -> f32| {
        let base = center.saturating_sub(radius);
        let skip = radius.saturating_sub(center);
        weights[skip..]
            .iter()
            .take(length - base)
            .enumerate()
            .map(|(index, weight)| value_at(base + index) * weight)
            .sum()
    };
    scratch
        .par_chunks_exact_mut(width)
        .zip(alpha.par_chunks_exact(width))
        .with_min_len(8)
        .for_each(|(out, row)| {
            for (column, value) in out.iter_mut().enumerate() {
                *value = sample(column, width, &|index| row[index]);
            }
        });
    alpha
        .par_chunks_exact_mut(width)
        .with_min_len(8)
        .enumerate()
        .for_each(|(row_index, row)| {
            for (column, value) in row.iter_mut().enumerate() {
                *value = sample(row_index, height, &|index| scratch[index * width + column]);
            }
        });
}

pub(crate) fn blend_source_over(dst: &mut [u8], src: [u8; 4]) {
    let sa = f32::from(src[3]) / 255.0;
    if sa <= 0.0 {
        return;
    }
    if src[3] == 255 || dst[3] == 0 {
        dst.copy_from_slice(&src);
        return;
    }

    let da = f32::from(dst[3]) / 255.0;
    let out_a = sa + da * (1.0 - sa);
    for channel in 0..3 {
        let sc = f32::from(src[channel]) / 255.0;
        let dc = f32::from(dst[channel]) / 255.0;
        let blended = (sc * sa + dc * da * (1.0 - sa)) / out_a;
        dst[channel] = (blended * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn apply_drop_shadow(
    image: &CapturedImage,
    offset_x: i32,
    offset_y: i32,
    blur_radius: u32,
    color: EffectColor,
    background: EffectColor,
) -> Result<CapturedImage, CoreError> {
    let src_w = i64::from(image.width());
    let src_h = i64::from(image.height());
    let ox = i64::from(offset_x);
    let oy = i64::from(offset_y);
    let blur = i64::from(blur_radius);

    let min_x = 0i64.min(ox - blur);
    let max_x = src_w.max(ox + src_w + blur);
    let min_y = 0i64.min(oy - blur);
    let max_y = src_h.max(oy + src_h + blur);

    let new_w = u32::try_from(max_x - min_x)
        .map_err(|_| CoreError::InvalidRegion("drop shadow width overflows u32".into()))?;
    let new_h = u32::try_from(max_y - min_y)
        .map_err(|_| CoreError::InvalidRegion("drop shadow height overflows u32".into()))?;

    validate_output_dimensions(new_w, new_h)?;
    let new_w_usize = new_w as usize;
    let total_bytes = checked_rgba_len(new_w, new_h)?;

    let img_x = (-min_x) as usize;
    let img_y = (-min_y) as usize;
    let shadow_x = (ox - min_x) as usize;
    let shadow_y = (oy - min_y) as usize;

    let src = image.rgba();
    let src_w_usize = image.width() as usize;
    let src_h_usize = image.height() as usize;
    let src_stride = src_w_usize * 4;
    let mut shadow_alpha = vec![0.0f32; total_bytes / 4];
    if color.a != 0 {
        let shadow_opacity = f32::from(color.a) / 255.0;
        shadow_alpha[shadow_y * new_w_usize..(shadow_y + src_h_usize) * new_w_usize]
            .par_chunks_exact_mut(new_w_usize)
            .zip(src.par_chunks_exact(src_stride))
            .with_min_len(8)
            .for_each(|(row, source)| {
                for (mask, pixel) in row[shadow_x..][..src_w_usize]
                    .iter_mut()
                    .zip(source.chunks_exact(4))
                {
                    *mask = (f32::from(pixel[3]) / 255.0) * shadow_opacity;
                }
            });
        blur_alpha_channel(&mut shadow_alpha, new_w_usize, blur_radius as usize);
    }

    let paint_shadow = |dest: &mut [u8], alpha: f32| {
        dest.copy_from_slice(&background.to_array());
        blend_source_over(
            dest,
            [color.r, color.g, color.b, (alpha * 255.0).round() as u8],
        );
    };
    let mut canvas = vec![0; total_bytes];
    canvas
        .par_chunks_exact_mut(new_w_usize * 4)
        .zip(shadow_alpha.par_chunks_exact(new_w_usize))
        .with_min_len(8)
        .enumerate()
        .for_each(|(row_index, (row, mask))| {
            if let Some(source_y) = row_index
                .checked_sub(img_y)
                .filter(|&row_index| row_index < src_h_usize)
            {
                let source = &src[source_y * src_stride..][..src_stride];
                let (left, middle_and_right) = row.split_at_mut(img_x * 4);
                let (middle, right) = middle_and_right.split_at_mut(src_stride);
                let (left_mask, rest_mask) = mask.split_at(img_x);
                let (middle_mask, right_mask) = rest_mask.split_at(src_w_usize);
                for (dest, &alpha) in left
                    .chunks_exact_mut(4)
                    .zip(left_mask)
                    .chain(right.chunks_exact_mut(4).zip(right_mask))
                {
                    paint_shadow(dest, alpha);
                }
                for ((dest, &alpha), pixel) in middle
                    .chunks_exact_mut(4)
                    .zip(middle_mask)
                    .zip(source.chunks_exact(4))
                {
                    if pixel[3] == 255 {
                        dest.copy_from_slice(pixel);
                    } else {
                        paint_shadow(dest, alpha);
                        blend_source_over(dest, [pixel[0], pixel[1], pixel[2], pixel[3]]);
                    }
                }
            } else {
                for (dest, &alpha) in row.chunks_exact_mut(4).zip(mask) {
                    paint_shadow(dest, alpha);
                }
            }
        });

    CapturedImage::from_rgba(new_w, new_h, canvas)
}

fn apply_resize(
    image: &CapturedImage,
    width: u32,
    height: u32,
    keep_aspect: bool,
) -> Result<CapturedImage, CoreError> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidRegion(format!(
            "resize target dimensions must be non-zero, got {width}x{height}"
        )));
    }

    let (target_w, target_h) = if keep_aspect {
        let scale_w = f64::from(width) / f64::from(image.width());
        let scale_h = f64::from(height) / f64::from(image.height());
        let scale = scale_w.min(scale_h);
        let scaled_w = (f64::from(image.width()) * scale).round().max(1.0) as u32;
        let scaled_h = (f64::from(image.height()) * scale).round().max(1.0) as u32;
        (scaled_w.min(width), scaled_h.min(height))
    } else {
        (width, height)
    };

    validate_output_dimensions(target_w, target_h)?;
    validate_output_dimensions(image.width(), target_h)?;
    resize_lanczos(image, target_w, target_h)
}

fn map_pixels(
    image: &CapturedImage,
    transform: impl Fn(&mut [u8]) + Sync + Send,
) -> Result<CapturedImage, CoreError> {
    let mut out = image.rgba().to_vec();
    out.par_chunks_exact_mut(4)
        .with_min_len(4096)
        .for_each(transform);
    CapturedImage::from_rgba(image.width(), image.height(), out)
}

fn luminance(pixel: &[u8]) -> f64 {
    0.299 * f64::from(pixel[0]) + 0.587 * f64::from(pixel[1]) + 0.114 * f64::from(pixel[2])
}

fn apply_grayscale(image: &CapturedImage) -> Result<CapturedImage, CoreError> {
    map_pixels(image, |pixel| {
        let luma = luminance(pixel).round() as u8;
        pixel[..3].fill(luma);
    })
}

fn apply_invert(image: &CapturedImage) -> Result<CapturedImage, CoreError> {
    map_pixels(image, |pixel| {
        pixel[0] = 255 - pixel[0];
        pixel[1] = 255 - pixel[1];
        pixel[2] = 255 - pixel[2];
    })
}

fn apply_brightness_contrast(
    image: &CapturedImage,
    brightness: i16,
    contrast: i16,
) -> Result<CapturedImage, CoreError> {
    let b = f64::from(brightness.clamp(-100, 100));
    let c = f64::from(contrast.clamp(-100, 100)) * 2.55;
    let b_offset = b * 2.55;
    let factor = (259.0 * (c + 255.0)) / (255.0 * (259.0 - c));

    let mut lut = [0u8; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        let adjusted = ((i as f64 + b_offset - 128.0) * factor + 128.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        *entry = adjusted;
    }

    map_pixels(image, |pixel| {
        pixel[0] = lut[usize::from(pixel[0])];
        pixel[1] = lut[usize::from(pixel[1])];
        pixel[2] = lut[usize::from(pixel[2])];
    })
}

pub(crate) fn unit_interval(value: f32) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        f64::from(value.clamp(0.0, 1.0))
    }
}

fn apply_colorize(
    image: &CapturedImage,
    color: EffectColor,
    strength: f32,
) -> Result<CapturedImage, CoreError> {
    let strength = unit_interval(strength);
    if strength == 0.0 {
        return Ok(image.clone());
    }
    let tint = [color.r, color.g, color.b].map(|component| f64::from(component) / 255.0);
    map_pixels(image, |pixel| {
        let luma = luminance(pixel);
        for (channel, tint) in pixel[..3].iter_mut().zip(tint) {
            *channel =
                (f64::from(*channel) * (1.0 - strength) + luma * tint * strength).round() as u8;
        }
    })
}

fn apply_slice(
    image: &CapturedImage,
    min_height: u32,
    max_height: u32,
    shift: i32,
) -> Result<CapturedImage, CoreError> {
    let width = image.width();
    let height = image.height();
    let limit = i64::from(width - 1);
    let shift = i64::from(shift).clamp(-limit, limit);
    if shift == 0 {
        return Ok(image.clone());
    }
    let minimum = min_height.clamp(1, height);
    let maximum = max_height.clamp(minimum, height);
    let stride = width as usize * 4;
    let offset = shift.unsigned_abs() as usize * 4;
    let mut out = vec![0; image.rgba().len()];
    let mut start = 0;
    let mut seed = 0x41c6_4e6du32;
    let mut shift_right = shift > 0;
    while start < height {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let rows = (minimum + seed % (maximum - minimum + 1)).min(height - start);
        let band = start as usize * stride..(start + rows) as usize * stride;
        for (dest, source) in out[band.clone()]
            .chunks_exact_mut(stride)
            .zip(image.rgba()[band].chunks_exact(stride))
        {
            if shift_right {
                dest[offset..].copy_from_slice(&source[..stride - offset]);
            } else {
                dest[..stride - offset].copy_from_slice(&source[offset..]);
            }
        }
        start += rows;
        shift_right = !shift_right;
    }

    CapturedImage::from_rgba(width, height, out)
}

fn pixel_within_tolerance(pixel: [u8; 4], reference: [u8; 4], tolerance: u8) -> bool {
    pixel[0].abs_diff(reference[0]) <= tolerance
        && pixel[1].abs_diff(reference[1]) <= tolerance
        && pixel[2].abs_diff(reference[2]) <= tolerance
        && pixel[3].abs_diff(reference[3]) <= tolerance
}

fn apply_auto_crop(image: &CapturedImage, tolerance: u8) -> Result<CapturedImage, CoreError> {
    let src = image.rgba();
    let reference = [src[0], src[1], src[2], src[3]];
    let w = image.width();
    let h = image.height();

    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for y in 0..h {
        for x in 0..w {
            let offset = ((y as usize) * (w as usize) + (x as usize)) * 4;
            let px = [
                src[offset],
                src[offset + 1],
                src[offset + 2],
                src[offset + 3],
            ];
            if !pixel_within_tolerance(px, reference, tolerance) {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }

    if !found {
        return image.crop(Region::new(0, 0, 1, 1)?);
    }

    let crop_x = i32::try_from(min_x)
        .map_err(|_| CoreError::InvalidRegion("auto-crop x offset overflows i32".into()))?;
    let crop_y = i32::try_from(min_y)
        .map_err(|_| CoreError::InvalidRegion("auto-crop y offset overflows i32".into()))?;
    let region = Region::new(crop_x, crop_y, max_x - min_x + 1, max_y - min_y + 1)?;
    image.crop(region)
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod regression_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_image(width: u32, height: u32, color: EffectColor) -> CapturedImage {
        let rgba = color
            .to_array()
            .repeat((width as usize) * (height as usize));
        CapturedImage::from_rgba(width, height, rgba).expect("valid test image")
    }

    fn pixel_at(image: &CapturedImage, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y as usize) * (image.width() as usize) + (x as usize)) * 4;
        let rgba = image.rgba();
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    }

    #[test]
    fn oversized_effect_outputs_return_errors_without_allocating() {
        let img = solid_image(4, 3, EffectColor::WHITE);
        for effect in [
            ImageEffect::CanvasPadding {
                top: 100_000,
                bottom: 100_000,
                left: 0,
                right: 0,
                color: EffectColor::TRANSPARENT,
            },
            ImageEffect::Border {
                width: 100_000,
                color: EffectColor::BLACK,
            },
            ImageEffect::DropShadow {
                offset_x: 0,
                offset_y: 0,
                blur_radius: 100_000,
                color: EffectColor::BLACK,
                background: EffectColor::TRANSPARENT,
            },
            ImageEffect::Resize {
                width: 50_000,
                height: 50_000,
                keep_aspect: true,
            },
            ImageEffect::Resize {
                width: 8_001,
                height: 8_000,
                keep_aspect: false,
            },
        ] {
            assert!(matches!(
                apply_effects(&img, &[effect]),
                Err(CoreError::InvalidRegion(message)) if message.contains("exceed the limits")
            ));
        }
    }

    #[test]
    fn effect_pipeline_checks_dimensions_before_cloning_the_input() {
        let img = solid_image(1, MAX_EFFECT_DIMENSION + 1, EffectColor::WHITE);
        assert!(matches!(
            apply_effects(&img, &[]),
            Err(CoreError::InvalidRegion(_))
        ));
        assert!(matches!(
            apply_effect(&img, &ImageEffect::Invert),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn border_expands_uniformly_and_preserves_inner_pixels() {
        let img = solid_image(4, 3, EffectColor::rgb(10, 20, 30));
        let border_color = EffectColor::rgb(200, 100, 50);
        let out = apply_effect(
            &img,
            &ImageEffect::Border {
                width: 2,
                color: border_color,
            },
        )
        .expect("border succeeds");

        assert_eq!(out.width(), 8);
        assert_eq!(out.height(), 7);
        assert_eq!(pixel_at(&out, 0, 0), border_color.to_array());
        assert_eq!(pixel_at(&out, 7, 6), border_color.to_array());
        assert_eq!(pixel_at(&out, 2, 2), [10, 20, 30, 255]);
        assert_eq!(pixel_at(&out, 5, 4), [10, 20, 30, 255]);

        let zero = apply_effect(
            &img,
            &ImageEffect::Border {
                width: 0,
                color: border_color,
            },
        )
        .expect("zero border succeeds");
        assert_eq!(zero, img);
    }

    #[test]
    fn canvas_padding_applies_asymmetric_margins() {
        let img = solid_image(3, 2, EffectColor::rgb(255, 0, 0));
        let pad_color = EffectColor::rgba(0, 255, 0, 128);
        let out = apply_effect(
            &img,
            &ImageEffect::CanvasPadding {
                top: 1,
                right: 4,
                bottom: 2,
                left: 3,
                color: pad_color,
            },
        )
        .expect("padding succeeds");

        assert_eq!(out.width(), 10);
        assert_eq!(out.height(), 5);
        assert_eq!(pixel_at(&out, 0, 0), pad_color.to_array());
        assert_eq!(pixel_at(&out, 2, 1), pad_color.to_array());
        assert_eq!(pixel_at(&out, 3, 1), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&out, 5, 2), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&out, 6, 2), pad_color.to_array());
        assert_eq!(pixel_at(&out, 3, 3), pad_color.to_array());
    }

    #[test]
    fn rounded_corners_clears_outer_corner_alpha_and_keeps_center_opaque() {
        let img = solid_image(20, 20, EffectColor::rgb(80, 120, 200));
        let out = apply_effect(&img, &ImageEffect::RoundedCorners { radius: 6 })
            .expect("rounded corners succeeds");

        assert_eq!(out.width(), 20);
        assert_eq!(out.height(), 20);
        assert_eq!(pixel_at(&out, 0, 0)[3], 0);
        assert_eq!(pixel_at(&out, 19, 0)[3], 0);
        assert_eq!(pixel_at(&out, 0, 19)[3], 0);
        assert_eq!(pixel_at(&out, 19, 19)[3], 0);
        assert_eq!(pixel_at(&out, 10, 10), [80, 120, 200, 255]);
        assert_eq!(pixel_at(&out, 10, 0)[3], 255);
        assert_eq!(pixel_at(&out, 0, 10)[3], 255);

        let has_antialiased_edge = (0..6).any(|y| {
            (0..6).any(|x| {
                let a = pixel_at(&out, x, y)[3];
                a > 0 && a < 255
            })
        });
        assert!(has_antialiased_edge);
    }

    #[test]
    fn drop_shadow_expands_canvas_draws_blurred_shadow_and_composites_source() {
        let img = solid_image(6, 6, EffectColor::rgb(255, 0, 0));
        let out = apply_effect(
            &img,
            &ImageEffect::DropShadow {
                offset_x: 3,
                offset_y: 4,
                blur_radius: 2,
                color: EffectColor::rgba(0, 0, 0, 180),
                background: EffectColor::WHITE,
            },
        )
        .expect("drop shadow succeeds");

        assert_eq!(out.width(), 11);
        assert_eq!(out.height(), 12);
        assert_eq!(pixel_at(&out, 2, 2), [255, 0, 0, 255]);

        let shadow_px = pixel_at(&out, 7, 8);
        assert_eq!(shadow_px[3], 255);
        assert!(shadow_px[0] < 255);
        assert_eq!(shadow_px[0], shadow_px[1]);
        assert_eq!(shadow_px[1], shadow_px[2]);

        let top_right = pixel_at(&out, 10, 0);
        assert!(top_right[0] > shadow_px[0]);

        let neg = apply_effect(
            &img,
            &ImageEffect::DropShadow {
                offset_x: -3,
                offset_y: -2,
                blur_radius: 1,
                color: EffectColor::rgba(0, 0, 0, 200),
                background: EffectColor::TRANSPARENT,
            },
        )
        .expect("negative offset drop shadow succeeds");
        assert_eq!(neg.width(), 10);
        assert_eq!(neg.height(), 9);
        assert_eq!(pixel_at(&neg, 6, 5), [255, 0, 0, 255]);
        assert!(pixel_at(&neg, 1, 1)[3] > 0);
    }

    #[test]
    fn resize_respects_aspect_ratio_and_rejects_zero_dimensions() {
        let img = solid_image(20, 10, EffectColor::rgb(30, 144, 255));

        let err_zero = apply_effect(
            &img,
            &ImageEffect::Resize {
                width: 0,
                height: 0,
                keep_aspect: false,
            },
        );
        assert!(matches!(err_zero, Err(CoreError::InvalidRegion(_))));

        let err_partial_zero = apply_effect(
            &img,
            &ImageEffect::Resize {
                width: 10,
                height: 0,
                keep_aspect: true,
            },
        );
        assert!(matches!(err_partial_zero, Err(CoreError::InvalidRegion(_))));

        let keep = apply_effect(
            &img,
            &ImageEffect::Resize {
                width: 10,
                height: 10,
                keep_aspect: true,
            },
        )
        .expect("aspect-preserving resize succeeds");
        assert_eq!(keep.width(), 10);
        assert_eq!(keep.height(), 5);
        assert_eq!(pixel_at(&keep, 5, 2), [30, 144, 255, 255]);

        let stretch = apply_effect(
            &img,
            &ImageEffect::Resize {
                width: 8,
                height: 12,
                keep_aspect: false,
            },
        )
        .expect("free resize succeeds");
        assert_eq!(stretch.width(), 8);
        assert_eq!(stretch.height(), 12);
    }

    #[test]
    fn grayscale_computes_luminance_and_preserves_alpha() {
        let rgba = vec![
            255, 0, 0, 200, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 255,
        ];
        let img = CapturedImage::from_rgba(4, 1, rgba).expect("valid 4x1 image");
        let out = apply_effect(&img, &ImageEffect::Grayscale).expect("grayscale succeeds");

        assert_eq!(pixel_at(&out, 0, 0), [76, 76, 76, 200]);
        assert_eq!(pixel_at(&out, 1, 0), [150, 150, 150, 128]);
        assert_eq!(pixel_at(&out, 2, 0), [29, 29, 29, 64]);
        assert_eq!(pixel_at(&out, 3, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn invert_flips_rgb_and_preserves_alpha() {
        let img = CapturedImage::from_rgba(2, 1, vec![10, 100, 250, 180, 0, 255, 128, 0])
            .expect("valid 2x1 image");
        let out = apply_effect(&img, &ImageEffect::Invert).expect("invert succeeds");

        assert_eq!(pixel_at(&out, 0, 0), [245, 155, 5, 180]);
        assert_eq!(pixel_at(&out, 1, 0), [255, 0, 127, 0]);
    }

    #[test]
    fn brightness_contrast_adjusts_values_and_clamps_extremes() {
        let img = CapturedImage::from_rgba(2, 1, vec![60, 128, 200, 210, 100, 150, 50, 90])
            .expect("valid 2x1 image");

        let identity = apply_effect(
            &img,
            &ImageEffect::BrightnessContrast {
                brightness: 0,
                contrast: 0,
            },
        )
        .expect("identity succeeds");
        assert_eq!(identity, img);

        let brighter = apply_effect(
            &img,
            &ImageEffect::BrightnessContrast {
                brightness: 25,
                contrast: 0,
            },
        )
        .expect("brighter succeeds");
        assert!(pixel_at(&brighter, 0, 0)[0] > 60);
        assert_eq!(pixel_at(&brighter, 0, 0)[3], 210);

        let flat = apply_effect(
            &img,
            &ImageEffect::BrightnessContrast {
                brightness: 0,
                contrast: -200,
            },
        )
        .expect("clamped low contrast succeeds");
        assert_eq!(pixel_at(&flat, 0, 0), [128, 128, 128, 210]);
        assert_eq!(pixel_at(&flat, 1, 0), [128, 128, 128, 90]);

        let max_bright = apply_effect(
            &img,
            &ImageEffect::BrightnessContrast {
                brightness: 500,
                contrast: 0,
            },
        )
        .expect("clamped max brightness succeeds");
        assert_eq!(pixel_at(&max_bright, 0, 0), [255, 255, 255, 210]);
    }

    #[test]
    fn auto_crop_trims_uniform_and_tolerant_margins_and_keeps_at_least_1x1() {
        let mut rgba = [250, 250, 250, 255].repeat(8 * 6);
        rgba[4] = 247;
        rgba[5] = 248;
        rgba[6] = 252;

        for y in 1..3 {
            for x in 2..5 {
                let idx = (y * 8 + x) * 4;
                rgba[idx..idx + 4].copy_from_slice(&[20, 40, 220, 255]);
            }
        }

        let img = CapturedImage::from_rgba(8, 6, rgba).expect("valid 8x6 image");
        let cropped =
            apply_effect(&img, &ImageEffect::AutoCrop { tolerance: 5 }).expect("crop succeeds");

        assert_eq!(cropped.width(), 3);
        assert_eq!(cropped.height(), 2);
        assert_eq!(pixel_at(&cropped, 0, 0), [20, 40, 220, 255]);
        assert_eq!(pixel_at(&cropped, 2, 1), [20, 40, 220, 255]);

        let solid = solid_image(5, 4, EffectColor::rgb(42, 42, 42));
        let single =
            apply_effect(&solid, &ImageEffect::AutoCrop { tolerance: 0 }).expect("solid crop");
        assert_eq!(single.width(), 1);
        assert_eq!(single.height(), 1);
        assert_eq!(pixel_at(&single, 0, 0), [42, 42, 42, 255]);
    }

    #[test]
    fn apply_effects_chains_multiple_effects_in_order() {
        let img = solid_image(10, 10, EffectColor::rgb(255, 0, 0));
        let pipeline = vec![
            ImageEffect::Grayscale,
            ImageEffect::Invert,
            ImageEffect::Resize {
                width: 6,
                height: 6,
                keep_aspect: false,
            },
            ImageEffect::Border {
                width: 2,
                color: EffectColor::BLACK,
            },
        ];

        let out = apply_effects(&img, &pipeline).expect("chained effects succeed");
        assert_eq!(out.width(), 10);
        assert_eq!(out.height(), 10);
        assert_eq!(pixel_at(&out, 0, 0), [0, 0, 0, 255]);
        assert_eq!(pixel_at(&out, 5, 5), [179, 179, 179, 255]);
    }
}
