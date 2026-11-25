use image::{RgbaImage, imageops::FilterType};

use super::*;

fn image_from_pixels(
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) -> CapturedImage {
    let mut rgba = Vec::new();
    for row_index in 0..height {
        for column in 0..width {
            rgba.extend(pixel(column, row_index));
        }
    }
    CapturedImage::from_rgba(width, height, rgba).unwrap()
}

fn sample(image: &CapturedImage, column: u32, row_index: u32) -> &[u8] {
    let offset = (row_index as usize * image.width() as usize + column as usize) * 4;
    &image.rgba()[offset..offset + 4]
}

#[test]
fn text_watermark_scales_at_all_nine_anchors() {
    use WatermarkAnchor::*;

    let anchors = [
        (TopLeft, 3, 3),
        (TopCenter, 15, 3),
        (TopRight, 27, 3),
        (MiddleLeft, 3, 9),
        (Center, 15, 9),
        (MiddleRight, 27, 9),
        (BottomLeft, 3, 16),
        (BottomCenter, 15, 16),
        (BottomRight, 27, 16),
    ];
    let source = image_from_pixels(40, 33, |_, _| [0; 4]);
    for (position, left, top) in anchors {
        let effect = ImageEffect::WatermarkText {
            text: "i".into(),
            position,
            color: RgbaColor::rgba(240, 100, 10, 200),
            opacity: 0.5,
            scale: 2,
            margin: 3,
            shadow: false,
        };
        let rendered = apply_effect(&source, &effect).unwrap();
        let expected = image_from_pixels(40, 33, |column, row_index| {
            let within =
                (left..left + 10).contains(&column) && (top..top + 14).contains(&row_index);
            if within
                && (row_index < top + 2
                    || row_index >= top + 12
                    || (left + 4..left + 6).contains(&column))
            {
                [240, 100, 10, 100]
            } else {
                [0; 4]
            }
        });
        assert_eq!(rendered, expected, "{position:?}");
    }
}

#[test]
fn text_watermark_opacity_and_shadow_respect_alpha() {
    let source = image_from_pixels(12, 16, |_, _| [0; 4]);
    let effect = |opacity, scale, shadow, margin| ImageEffect::WatermarkText {
        text: "I\nI".into(),
        position: WatermarkAnchor::TopLeft,
        color: RgbaColor::rgba(230, 130, 30, 200),
        opacity,
        scale,
        margin,
        shadow,
    };
    let rendered = apply_effect(&source, &effect(0.5, 0, true, 0)).unwrap();
    assert_eq!(sample(&rendered, 0, 0), [230, 130, 30, 100]);
    assert_eq!(sample(&rendered, 5, 1), [0, 0, 0, 50]);
    assert_eq!(sample(&rendered, 0, 7), [0; 4]);
    assert_eq!(sample(&rendered, 0, 8), [230, 130, 30, 100]);
    for opacity in [0.0, -1.0, f32::NAN] {
        assert_eq!(
            apply_effect(&source, &effect(opacity, 1, true, 0)).unwrap(),
            source
        );
    }
    let clipped = apply_effect(&source, &effect(2.0, u32::MAX, true, u32::MAX)).unwrap();
    assert_eq!(sample(&clipped, 11, 15), [230, 130, 30, 200]);
}

#[test]
fn image_watermark_uses_straight_alpha_and_clips_to_anchor() {
    let source = image_from_pixels(3, 2, |column, _| {
        if column == 0 {
            [0, 0, 240, 128]
        } else {
            [0, 0, 240, 255]
        }
    });
    let effect = ImageEffect::ImageWatermark {
        rgba: [200, 0, 0, 128].repeat(2),
        width: 2,
        height: 1,
        position: WatermarkAnchor::BottomLeft,
        opacity: 0.5,
        margin: 0,
    };
    let rendered = apply_effect(&source, &effect).unwrap();
    assert_eq!(sample(&rendered, 0, 1), [80, 0, 144, 160]);
    assert_eq!(sample(&rendered, 1, 1), [50, 0, 180, 255]);
    assert_eq!(sample(&rendered, 0, 0), sample(&source, 0, 0));
    assert_eq!(sample(&rendered, 2, 1), sample(&source, 2, 1));

    let source = image_from_pixels(1, 1, |_, _| [0; 4]);
    let effect = ImageEffect::ImageWatermark {
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        width: 2,
        height: 1,
        position: WatermarkAnchor::BottomRight,
        opacity: 1.0,
        margin: u32::MAX,
    };
    assert_eq!(
        apply_effect(&source, &effect).unwrap().rgba(),
        [0, 255, 0, 255]
    );
}

#[test]
fn image_watermark_rejects_bad_buffer_even_when_disabled_by_opacity() {
    let source = image_from_pixels(1, 1, |_, _| [0; 4]);
    for (width, height, byte_count) in [
        (0, 1, 0),
        (1, 0, 0),
        (1, 2, 7),
        (1, 1, 5),
        (u32::MAX, u32::MAX, 0),
    ] {
        let effect = ImageEffect::ImageWatermark {
            rgba: vec![0; byte_count],
            width,
            height,
            position: WatermarkAnchor::Center,
            opacity: 0.0,
            margin: 0,
        };
        assert!(matches!(
            apply_effect(&source, &effect),
            Err(CoreError::InvalidImageBuffer { .. })
        ));
    }
}

#[test]
fn colorize_interpolates_toward_luminance_tint_without_changing_alpha() {
    let source = image_from_pixels(1, 1, |_, _| [200, 100, 50, 77]);
    let effect = |strength| ImageEffect::Colorize {
        color: RgbaColor::rgba(255, 128, 0, 0),
        strength,
    };
    for strength in [0.0, -1.0, f32::NAN] {
        assert_eq!(apply_effect(&source, &effect(strength)).unwrap(), source);
    }
    for strength in [1.0, 2.0] {
        assert_eq!(
            apply_effect(&source, &effect(strength)).unwrap().rgba(),
            [124, 62, 0, 77]
        );
    }
    assert_eq!(
        apply_effect(&source, &effect(0.5)).unwrap().rgba(),
        [162, 81, 25, 77]
    );
}

#[test]
fn slice_uses_deterministic_bands_and_clamps_extremes() {
    let source = image_from_pixels(4, 7, |column, row_index| {
        [column as u8, row_index as u8, 99, 255]
    });
    let effect = |min_height, max_height, shift| ImageEffect::Slice {
        min_height,
        max_height,
        shift,
    };
    let shifted = apply_effect(&source, &effect(2, 2, 1)).unwrap();
    for row_index in 0..7 {
        let right = row_index / 2 % 2 == 0;
        for column in 0..4 {
            let expected = if right && column != 0 {
                sample(&source, column - 1, row_index)
            } else if !right && column != 3 {
                sample(&source, column + 1, row_index)
            } else {
                &[0; 4]
            };
            assert_eq!(sample(&shifted, column, row_index), expected);
        }
    }
    let random_bands = effect(1, 4, 2);
    assert_eq!(
        apply_effect(&source, &random_bands).unwrap(),
        apply_effect(&source, &random_bands).unwrap()
    );
    assert_eq!(
        apply_effect(&source, &effect(0, u32::MAX, 0)).unwrap(),
        source
    );
    let clamped = apply_effect(&source, &effect(u32::MAX, 0, i32::MIN)).unwrap();
    for row_index in 0..7 {
        assert_eq!(
            sample(&clamped, 0, row_index),
            sample(&source, 3, row_index)
        );
        assert_eq!(sample(&clamped, 1, row_index), [0; 4]);
    }
    let one_column = image_from_pixels(1, 3, |_, _| [12, 23, 34, 255]);
    assert_eq!(
        apply_effect(&one_column, &effect(0, 0, i32::MAX)).unwrap(),
        one_column
    );
}

#[test]
fn parallel_resize_matches_serial_lanczos_including_alpha() {
    let source = image_from_pixels(19, 17, |column, row_index| {
        [
            (column * 43 + row_index * 7) as u8,
            (row_index * 59) as u8,
            (column * row_index) as u8,
            (column * 13 + row_index * 33) as u8,
        ]
    });
    let rgba = RgbaImage::from_raw(19, 17, source.rgba().to_vec()).unwrap();
    for (width, height, keep_aspect, tw, th) in [
        (6, 9, false, 6, 9),
        (123, 73, false, 123, 73),
        (19, 17, false, 19, 17),
        (19, 8, false, 19, 8),
        (8, 17, false, 8, 17),
        (4, 16, true, 4, 4),
        (1, 1, false, 1, 1),
    ] {
        let resized = apply_resize(&source, width, height, keep_aspect).unwrap();
        let reference = image::imageops::resize(&rgba, tw, th, FilterType::Lanczos3);
        assert_eq!((resized.width(), resized.height()), (tw, th));
        assert_eq!(resized.rgba(), reference.as_raw(), "{width}x{height}");
    }
}

#[test]
fn parallel_color_filters_match_serial_reference() {
    let source = image_from_pixels(173, 107, |column, row_index| {
        [
            (column * 41 + row_index * 13) as u8,
            (column * 29) as u8,
            (row_index * 17) as u8,
            (row_index * column) as u8,
        ]
    });
    let factor = 259.0 * (37.0 * 2.55 + 255.0) / (255.0 * (259.0 - 37.0 * 2.55));
    let adjust =
        |component| ((f64::from(component) - 23.0 * 2.55 - 128.0) * factor + 128.0).round() as u8;
    let gray = apply_grayscale(&source).unwrap();
    let inverse = apply_invert(&source).unwrap();
    let adjusted = apply_brightness_contrast(&source, -23, 37).unwrap();
    for (id, pixel) in source.rgba().chunks_exact(4).enumerate() {
        let [red, green, blue, alpha]: [u8; 4] = pixel.try_into().unwrap();
        let gray_value =
            (0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue)).round()
                as u8;
        let index = id * 4..id * 4 + 4;
        assert_eq!(
            gray.rgba()[index.clone()],
            [gray_value, gray_value, gray_value, alpha]
        );
        assert_eq!(
            inverse.rgba()[index.clone()],
            [255 - red, 255 - green, 255 - blue, alpha]
        );
        assert_eq!(
            adjusted.rgba()[index],
            [adjust(red), adjust(green), adjust(blue), alpha]
        );
    }
}

#[test]
fn parallel_padding_and_corners_match_serial_for_asymmetric_and_tiny_images() {
    for (width, height, radius) in [(35, 59, 17), (9, 3, 20), (1, 1, 999), (1, 7, 2)] {
        let color = RgbaColor::rgba(8, 7, 6, 5);
        let source = image_from_pixels(width, height, |column, row_index| {
            [column as u8, row_index as u8, 123, 157]
        });
        for (top, right, bottom, left) in [(1, 4, 0, 7), (3, 3, 3, 3)] {
            let effect = if top == right {
                ImageEffect::Border { width: top, color }
            } else {
                ImageEffect::CanvasPadding {
                    top,
                    right,
                    bottom,
                    left,
                    color,
                }
            };
            let expected = image_from_pixels(
                width + left + right,
                height + top + bottom,
                |column, row_index| {
                    if (left..left + width).contains(&column)
                        && (top..top + height).contains(&row_index)
                    {
                        sample(&source, column - left, row_index - top)
                            .try_into()
                            .unwrap()
                    } else {
                        color.to_array()
                    }
                },
            );
            assert_eq!(apply_effect(&source, &effect).unwrap(), expected);
        }
        let radius_f = f64::from(radius)
            .min(f64::from(width) / 2.0)
            .min(f64::from(height) / 2.0);
        let expected = image_from_pixels(width, height, |column, row_index| {
            let center_x = f64::from(column) + 0.5;
            let center_y = f64::from(row_index) + 0.5;
            let distance = (center_x - center_x.clamp(radius_f, f64::from(width) - radius_f))
                .hypot(center_y - center_y.clamp(radius_f, f64::from(height) - radius_f));
            let alpha = (157.0 * (radius_f - distance).clamp(0.0, 1.0)).round() as u8;
            let in_corner = (center_x < radius_f || center_x > f64::from(width) - radius_f)
                && (center_y < radius_f || center_y > f64::from(height) - radius_f);
            [
                column as u8,
                row_index as u8,
                123,
                if in_corner { alpha } else { 157 },
            ]
        });
        assert_eq!(apply_rounded_corners(&source, radius).unwrap(), expected);
    }
}

fn serial_blur(source: &[f32], width: usize, passes: &[Vec<f32>]) -> Vec<f32> {
    let height = source.len() / width;
    let mut alpha = source.to_vec();
    for weights in passes {
        let radius = weights.len() / 2;
        for vertical in [false, true] {
            alpha = (0..source.len())
                .map(|index| {
                    let (column, row_index) = (index % width, index / width);
                    let (coordinate, length) = if vertical {
                        (row_index, height)
                    } else {
                        (column, width)
                    };
                    weights
                        .iter()
                        .enumerate()
                        .filter_map(|(offset, weight)| {
                            (coordinate + offset)
                                .checked_sub(radius)
                                .filter(|&value| value < length)
                                .map(|value| {
                                    weight
                                        * alpha[if vertical {
                                            value * width + column
                                        } else {
                                            row_index * width + value
                                        }]
                                })
                        })
                        .sum()
                })
                .collect();
        }
    }
    alpha
}

#[test]
fn running_shadow_blur_matches_direct_box_passes_and_small_gaussian() {
    let width = 71;
    let height = 70;
    let source: Vec<f32> = (0..width * height)
        .map(|index| {
            let (column, row_index) = (index % width, index / width);
            if (8..63).contains(&column) && (9..61).contains(&row_index) {
                ((column * 73 + row_index * 61) % 257) as f32 / 256.0
            } else {
                0.0
            }
        })
        .collect();

    for radius in [0usize, 1, 2, 3, 16, 40] {
        let passes = if radius <= 2 {
            let sigma = (radius as f32 / 2.5).max(0.5);
            let mut weights: Vec<_> = (0..2 * radius + 1)
                .map(|offset| {
                    let distance = offset.abs_diff(radius) as f32;
                    (-distance.powi(2) / (2.0 * sigma * sigma)).exp()
                })
                .collect();
            let sum: f32 = weights.iter().sum();
            weights.iter_mut().for_each(|weight| *weight /= sum);
            vec![weights]
        } else {
            (0..3)
                .map(|index| {
                    let length = 2 * ((radius + index) / 3) + 1;
                    vec![1.0 / length as f32; length]
                })
                .collect()
        };

        let expected = serial_blur(&source, width, &passes);
        let mut actual = source.clone();
        blur_alpha_channel(&mut actual, width, radius);
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 0.000_01),
            "radius {radius}"
        );
    }
}
