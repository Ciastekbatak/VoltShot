use rayon::prelude::*;
use voltshot_core::{CapturedImage, CoreError};

use crate::effects::checked_rgba_len;

struct Sample {
    start: usize,
    weights: Vec<f32>,
}

fn sinc(distance: f32) -> f32 {
    if distance == 0.0 {
        return 1.0;
    }
    let radians = distance * std::f32::consts::PI;
    radians.sin() / radians
}

fn lanczos_samples(source: u32, target: u32) -> Vec<Sample> {
    let ratio = source as f32 / target as f32;
    let step = ratio.max(1.0);
    let support = 3.0 * step;
    (0..target)
        .into_par_iter()
        .with_min_len(64)
        .map(|index| {
            let center = (index as f32 + 0.5) * ratio;
            let start = ((center - support).floor() as u32).min(source - 1);
            let end = ((center + support).ceil() as u32).clamp(start + 1, source);
            let mut weights: Vec<f32> = (start..end)
                .map(|sample| {
                    let distance = (sample as f32 - (center - 0.5)) / step;
                    if distance.abs() < 3.0 {
                        sinc(distance) * sinc(distance / 3.0)
                    } else {
                        0.0
                    }
                })
                .collect();
            let sum: f32 = weights.iter().sum();
            for weight in &mut weights {
                *weight /= sum;
            }
            Sample {
                start: start as usize,
                weights,
            }
        })
        .collect()
}

pub(crate) fn resize_lanczos(
    image: &CapturedImage,
    width: u32,
    height: u32,
) -> Result<CapturedImage, CoreError> {
    if width == image.width() && height == image.height() {
        return Ok(image.clone());
    }

    let output_len = checked_rgba_len(width, height)?;
    let temp_len = checked_rgba_len(image.width(), height)?;
    if temp_len > isize::MAX as usize / size_of::<f32>() {
        return Err(CoreError::InvalidRegion(
            "resize intermediate dimensions overflow buffer allocation".into(),
        ));
    }

    let vertical = lanczos_samples(image.height(), height);
    let horizontal = lanczos_samples(image.width(), width);
    let src_stride = image.width() as usize * 4;
    let mut temp = vec![0.0; temp_len];
    temp.par_chunks_exact_mut(src_stride)
        .zip(vertical.par_iter())
        .with_min_len(4)
        .for_each(|(row, sample)| {
            for (offset, &weight) in sample.weights.iter().enumerate() {
                let source = &image.rgba()[(sample.start + offset) * src_stride..][..src_stride];
                for (output, &value) in row.iter_mut().zip(source) {
                    *output += f32::from(value) * weight;
                }
            }
        });

    let mut out = vec![0; output_len];
    out.par_chunks_exact_mut(width as usize * 4)
        .zip(temp.par_chunks_exact(src_stride))
        .with_min_len(4)
        .for_each(|(row, source)| {
            for (pixel, sample) in row.chunks_exact_mut(4).zip(&horizontal) {
                let mut sum = [0.0; 4];
                let input = source[sample.start * 4..].chunks_exact(4);
                for (sample, &weight) in input.zip(&sample.weights) {
                    for (channel, value) in sum.iter_mut().zip(sample) {
                        *channel += value * weight;
                    }
                }
                for (channel, value) in pixel.iter_mut().zip(sum) {
                    *channel = value.round() as u8;
                }
            }
        });
    CapturedImage::from_rgba(width, height, out)
}
