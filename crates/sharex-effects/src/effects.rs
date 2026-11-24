use image::{RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use sharex_core::{CapturedImage, CoreError, Region};

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

/// Composable image transformation applied to a [`CapturedImage`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// Applies a single [`ImageEffect`] to an image and returns the transformed copy.
pub fn apply_effect(
    image: &CapturedImage,
    effect: &ImageEffect,
) -> Result<CapturedImage, CoreError> {
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
    }
}

/// Applies a sequence of [`ImageEffect`] transformations in order.
pub fn apply_effects(
    image: &CapturedImage,
    effects: &[ImageEffect],
) -> Result<CapturedImage, CoreError> {
    let mut current = image.clone();
    for effect in effects {
        current = apply_effect(&current, effect)?;
    }
    Ok(current)
}

fn checked_rgba_len(width: u32, height: u32) -> Result<usize, CoreError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            CoreError::InvalidRegion(format!(
                "image dimensions {width}x{height} overflow buffer allocation"
            ))
        })
}

fn fill_rgba_buffer(width: u32, height: u32, color: EffectColor) -> Result<Vec<u8>, CoreError> {
    let total_bytes = checked_rgba_len(width, height)?;
    let pixel = color.to_array();
    let mut buf = Vec::with_capacity(total_bytes);
    for _ in 0..(total_bytes / 4) {
        buf.extend_from_slice(&pixel);
    }
    Ok(buf)
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

    let mut out = fill_rgba_buffer(new_w, new_h, color)?;
    let src_stride = (image.width() as usize) * 4;
    let dst_stride = (new_w as usize) * 4;
    let left_bytes = (left as usize) * 4;
    let src = image.rgba();

    for y in 0..image.height() as usize {
        let src_offset = y * src_stride;
        let dst_offset = (y + top as usize) * dst_stride + left_bytes;
        out[dst_offset..dst_offset + src_stride]
            .copy_from_slice(&src[src_offset..src_offset + src_stride]);
    }

    CapturedImage::from_rgba(new_w, new_h, out)
}

fn apply_rounded_corners(image: &CapturedImage, radius: u32) -> Result<CapturedImage, CoreError> {
    if radius == 0 {
        return Ok(image.clone());
    }

    let w = image.width();
    let h = image.height();
    let wf = f64::from(w);
    let hf = f64::from(h);
    let r = f64::from(radius).min(wf / 2.0).min(hf / 2.0);
    let mut out = image.rgba().to_vec();

    for y in 0..h {
        let cy = f64::from(y) + 0.5;
        let corner_y = if cy < r {
            Some(r)
        } else if cy > hf - r {
            Some(hf - r)
        } else {
            None
        };

        let Some(ky) = corner_y else {
            continue;
        };

        for x in 0..w {
            let cx = f64::from(x) + 0.5;
            let corner_x = if cx < r {
                Some(r)
            } else if cx > wf - r {
                Some(wf - r)
            } else {
                None
            };

            let Some(kx) = corner_x else {
                continue;
            };

            let dist = (cx - kx).hypot(cy - ky);
            if dist <= r - 1.0 {
                continue;
            }

            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            if dist >= r {
                out[idx + 3] = 0;
            } else {
                let coverage = (r - dist).clamp(0.0, 1.0);
                out[idx + 3] = (f64::from(out[idx + 3]) * coverage).round() as u8;
            }
        }
    }

    CapturedImage::from_rgba(w, h, out)
}

fn gaussian_kernel(radius: u32) -> Vec<f32> {
    let sigma = (radius as f32 / 2.5).max(0.5);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let r = radius as i32;
    let mut kernel = Vec::with_capacity((radius as usize) * 2 + 1);
    let mut sum = 0.0f32;

    for d in -r..=r {
        let weight = (-((d * d) as f32) / two_sigma_sq).exp();
        kernel.push(weight);
        sum += weight;
    }

    if sum > 0.0 {
        for weight in &mut kernel {
            *weight /= sum;
        }
    }

    kernel
}

fn blur_alpha_channel(alpha: &[f32], width: usize, height: usize, radius: u32) -> Vec<f32> {
    if radius == 0 {
        return alpha.to_vec();
    }

    let kernel = gaussian_kernel(radius);
    let r = radius as isize;
    let mut temp = vec![0.0f32; width * height];
    let mut out = vec![0.0f32; width * height];

    for y in 0..height {
        let row_offset = y * width;
        for x in 0..width {
            let mut acc = 0.0f32;
            for (k_idx, &weight) in kernel.iter().enumerate() {
                let sx = x as isize + (k_idx as isize - r);
                if sx >= 0 && (sx as usize) < width {
                    acc += alpha[row_offset + sx as usize] * weight;
                }
            }
            temp[row_offset + x] = acc;
        }
    }

    for y in 0..height {
        let row_offset = y * width;
        for x in 0..width {
            let mut acc = 0.0f32;
            for (k_idx, &weight) in kernel.iter().enumerate() {
                let sy = y as isize + (k_idx as isize - r);
                if sy >= 0 && (sy as usize) < height {
                    acc += temp[(sy as usize) * width + x] * weight;
                }
            }
            out[row_offset + x] = acc;
        }
    }

    out
}

fn blend_source_over(dst: &mut [u8; 4], src: [u8; 4]) {
    let sa = f32::from(src[3]) / 255.0;
    if sa <= 0.0 {
        return;
    }
    if src[3] == 255 {
        *dst = src;
        return;
    }

    let da = f32::from(dst[3]) / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = [0, 0, 0, 0];
        return;
    }

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

    let new_w_usize = new_w as usize;
    let new_h_usize = new_h as usize;
    let total_pixels = new_w_usize.checked_mul(new_h_usize).ok_or_else(|| {
        CoreError::InvalidRegion("drop shadow canvas pixel count overflows usize".into())
    })?;

    let img_x = (-min_x) as usize;
    let img_y = (-min_y) as usize;
    let shadow_x = (ox - min_x) as usize;
    let shadow_y = (oy - min_y) as usize;

    let mut shadow_alpha = vec![0.0f32; total_pixels];
    let shadow_opacity = f32::from(color.a) / 255.0;
    let src = image.rgba();
    let src_w_usize = image.width() as usize;
    let src_h_usize = image.height() as usize;

    for y in 0..src_h_usize {
        for x in 0..src_w_usize {
            let src_a = f32::from(src[(y * src_w_usize + x) * 4 + 3]) / 255.0;
            shadow_alpha[(shadow_y + y) * new_w_usize + (shadow_x + x)] = src_a * shadow_opacity;
        }
    }

    let blurred_alpha = blur_alpha_channel(&shadow_alpha, new_w_usize, new_h_usize, blur_radius);
    let mut canvas = fill_rgba_buffer(new_w, new_h, background)?;

    for (idx, &alpha) in blurred_alpha.iter().enumerate() {
        let a_u8 = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        if a_u8 == 0 {
            continue;
        }
        let offset = idx * 4;
        let mut dst = [
            canvas[offset],
            canvas[offset + 1],
            canvas[offset + 2],
            canvas[offset + 3],
        ];
        blend_source_over(&mut dst, [color.r, color.g, color.b, a_u8]);
        canvas[offset..offset + 4].copy_from_slice(&dst);
    }

    for y in 0..src_h_usize {
        for x in 0..src_w_usize {
            let src_offset = (y * src_w_usize + x) * 4;
            let dst_offset = ((img_y + y) * new_w_usize + (img_x + x)) * 4;
            let mut dst = [
                canvas[dst_offset],
                canvas[dst_offset + 1],
                canvas[dst_offset + 2],
                canvas[dst_offset + 3],
            ];
            let src_px = [
                src[src_offset],
                src[src_offset + 1],
                src[src_offset + 2],
                src[src_offset + 3],
            ];
            blend_source_over(&mut dst, src_px);
            canvas[dst_offset..dst_offset + 4].copy_from_slice(&dst);
        }
    }

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

    let src = RgbaImage::from_raw(image.width(), image.height(), image.rgba().to_vec())
        .ok_or_else(|| CoreError::InvalidImageBuffer {
            width: image.width(),
            height: image.height(),
            expected: (image.width() as usize) * (image.height() as usize) * 4,
            actual: image.rgba().len(),
        })?;

    let resized = image::imageops::resize(&src, target_w, target_h, FilterType::Lanczos3);
    CapturedImage::from_rgba(target_w, target_h, resized.into_raw())
}

fn apply_grayscale(image: &CapturedImage) -> Result<CapturedImage, CoreError> {
    let mut out = image.rgba().to_vec();
    for pixel in out.chunks_exact_mut(4) {
        let luma = (0.299 * f64::from(pixel[0])
            + 0.587 * f64::from(pixel[1])
            + 0.114 * f64::from(pixel[2]))
        .round()
        .clamp(0.0, 255.0) as u8;
        pixel[0] = luma;
        pixel[1] = luma;
        pixel[2] = luma;
    }
    CapturedImage::from_rgba(image.width(), image.height(), out)
}

fn apply_invert(image: &CapturedImage) -> Result<CapturedImage, CoreError> {
    let mut out = image.rgba().to_vec();
    for pixel in out.chunks_exact_mut(4) {
        pixel[0] = 255 - pixel[0];
        pixel[1] = 255 - pixel[1];
        pixel[2] = 255 - pixel[2];
    }
    CapturedImage::from_rgba(image.width(), image.height(), out)
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

    let mut out = image.rgba().to_vec();
    for pixel in out.chunks_exact_mut(4) {
        pixel[0] = lut[usize::from(pixel[0])];
        pixel[1] = lut[usize::from(pixel[1])];
        pixel[2] = lut[usize::from(pixel[2])];
    }

    CapturedImage::from_rgba(image.width(), image.height(), out)
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
