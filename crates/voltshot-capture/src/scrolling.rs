use voltshot_core::{CapturedImage, CoreError};

/// Controls the overlap search between consecutive scrolling frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollStitchOptions {
    /// Minimum number of matching content rows required to accept an overlap.
    pub min_overlap_rows: u32,
    /// Largest overlap to search, relative to the shorter content band, in `(0, 1]`.
    pub max_overlap_fraction: f32,
    /// Maximum absolute difference allowed for each RGBA channel in a matching row.
    pub row_tolerance: u8,
    /// Fixed top rows to exclude from matching and keep only from the first frame.
    pub ignore_header_rows: u32,
    /// Fixed bottom rows to exclude from matching and keep only from the last unique frame.
    pub ignore_footer_rows: u32,
}

impl Default for ScrollStitchOptions {
    fn default() -> Self {
        Self {
            min_overlap_rows: 8,
            max_overlap_fraction: 0.95,
            row_tolerance: 0,
            ignore_header_rows: 0,
            ignore_footer_rows: 0,
        }
    }
}

/// A stitched scrolling image and its per-frame overlap diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollStitchResult {
    /// Combined top-to-bottom image with each sticky band included once.
    pub image: CapturedImage,
    /// Number of frames retained, including the first and excluding consecutive duplicates.
    pub frames_merged: usize,
    /// Overlap for each retained frame after the first; zero means the frame was appended.
    pub overlap_rows: Vec<u32>,
}

/// Stitches frames captured in downward scroll order, appending unmatched content as-is.
pub fn stitch_scrolling_frames(
    frames: &[CapturedImage],
    opts: &ScrollStitchOptions,
) -> Result<ScrollStitchResult, CoreError> {
    let first = frames
        .first()
        .ok_or_else(|| CoreError::CaptureFailed("no scrolling frames to stitch".into()))?;
    if frames.iter().any(|frame| frame.width() != first.width()) {
        return Err(CoreError::CaptureFailed(
            "scrolling frame widths differ".into(),
        ));
    }
    let fixed_rows = opts
        .ignore_header_rows
        .saturating_add(opts.ignore_footer_rows);
    if frames.iter().any(|frame| frame.height() <= fixed_rows) {
        return Err(CoreError::CaptureFailed(
            "sticky bands leave no scrolling content".into(),
        ));
    }
    if opts.min_overlap_rows == 0
        || !(0.0 < opts.max_overlap_fraction && opts.max_overlap_fraction <= 1.0)
    {
        return Err(CoreError::CaptureFailed(
            "invalid scrolling overlap limits".into(),
        ));
    }

    let stride = first.rgba().len() / first.height() as usize;
    let header_bytes = opts.ignore_header_rows as usize * stride;
    let footer_bytes = opts.ignore_footer_rows as usize * stride;
    let content = |frame: &CapturedImage| header_bytes..frame.rgba().len() - footer_bytes;

    let mut rgba = Vec::new();
    append(&mut rgba, &first.rgba()[..content(first).end])?;
    let mut overlap_rows = Vec::new();
    let mut previous = first;
    for frame in &frames[1..] {
        if frame == previous {
            continue;
        }
        let previous_rows = &previous.rgba()[content(previous)];
        let current_rows = &frame.rgba()[content(frame)];
        let overlap = find_overlap(previous_rows, current_rows, stride, opts);
        append(&mut rgba, &current_rows[overlap as usize * stride..])?;
        overlap_rows.push(overlap);
        previous = frame;
    }
    append(&mut rgba, &previous.rgba()[content(previous).end..])?;

    let height = u32::try_from(rgba.len() / stride)
        .map_err(|_| CoreError::CaptureFailed("stitched scrolling image is too tall".into()))?;
    Ok(ScrollStitchResult {
        image: CapturedImage::from_rgba(first.width(), height, rgba)?,
        frames_merged: overlap_rows.len() + 1,
        overlap_rows,
    })
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CoreError> {
    output.try_reserve(bytes.len()).map_err(|error| {
        CoreError::CaptureFailed(format!("cannot allocate stitched scrolling image: {error}"))
    })?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn find_overlap(previous: &[u8], current: &[u8], stride: usize, opts: &ScrollStitchOptions) -> u32 {
    let shorter_height = previous.len().min(current.len()) / stride;
    let max_overlap = (shorter_height as f64 * f64::from(opts.max_overlap_fraction)) as usize;
    for rows in (opts.min_overlap_rows as usize..=max_overlap).rev() {
        let len = rows * stride;
        let previous_tail = &previous[previous.len() - len..];
        let current_head = &current[..len];
        let sampled_rows = [0, rows - 1, rows / 2, rows / 4, rows - 1 - rows / 4];
        if sampled_rows.into_iter().all(|row| {
            let span = row * stride..(row + 1) * stride;
            matches_channels(
                &previous_tail[span.clone()],
                &current_head[span],
                opts.row_tolerance,
            )
        }) && matches_channels(previous_tail, current_head, opts.row_tolerance)
        {
            return rows as u32;
        }
    }
    0
}

fn matches_channels(left: &[u8], right: &[u8], tolerance: u8) -> bool {
    left == right
        || (tolerance != 0
            && left
                .iter()
                .zip(right)
                .all(|(&left, &right)| left.abs_diff(right) <= tolerance))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rows: &[u8]) -> CapturedImage {
        let rgba = rows
            .iter()
            .flat_map(|&row| [row, 0, 0, 255, 0, row, 0, 255])
            .collect();
        CapturedImage::from_rgba(2, rows.len() as u32, rgba).expect("valid test frame")
    }

    fn options() -> ScrollStitchOptions {
        ScrollStitchOptions {
            min_overlap_rows: 2,
            ..ScrollStitchOptions::default()
        }
    }

    #[test]
    fn stitches_three_frames_with_exact_pixels() {
        let frames = [
            frame(&[1, 2, 3, 4]),
            frame(&[3, 4, 5, 6, 7]),
            frame(&[5, 6, 7, 8]),
        ];
        let result = stitch_scrolling_frames(&frames, &options()).expect("stitch succeeds");

        assert_eq!(result.image, frame(&[1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(result.frames_merged, 3);
        assert_eq!(result.overlap_rows, [2, 3]);
    }

    #[test]
    fn keeps_only_the_first_header_and_last_footer() {
        let frames = [
            frame(&[100, 1, 2, 3, 4, 200]),
            frame(&[101, 3, 4, 5, 6, 201]),
        ];
        let opts = ScrollStitchOptions {
            ignore_header_rows: 1,
            ignore_footer_rows: 1,
            ..options()
        };
        let result = stitch_scrolling_frames(&frames, &opts).expect("stitch succeeds");

        assert_eq!(result.image, frame(&[100, 1, 2, 3, 4, 5, 6, 201]));
        assert_eq!(result.overlap_rows, [2]);
    }

    #[test]
    fn drops_duplicate_bottom_frames() {
        let bottom = frame(&[3, 4, 5, 6]);
        let frames = [frame(&[1, 2, 3, 4]), bottom.clone(), bottom];
        let result = stitch_scrolling_frames(&frames, &options()).expect("stitch succeeds");

        assert_eq!(result.image, frame(&[1, 2, 3, 4, 5, 6]));
        assert_eq!(result.frames_merged, 2);
        assert_eq!(result.overlap_rows, [2]);
    }

    #[test]
    fn appends_a_non_overlapping_frame() {
        let frames = [frame(&[1, 2, 3]), frame(&[4, 5, 6])];
        let result = stitch_scrolling_frames(&frames, &options()).expect("stitch succeeds");

        assert_eq!(result.image, frame(&[1, 2, 3, 4, 5, 6]));
        assert_eq!(result.overlap_rows, [0]);
    }

    #[test]
    fn exact_verification_rejects_a_difference_between_sampled_rows() {
        let frames = [
            frame(&[1, 2, 3, 4, 5, 6, 7, 8]),
            frame(&[2, 3, 99, 5, 6, 7, 8, 9]),
        ];
        let opts = ScrollStitchOptions {
            min_overlap_rows: 7,
            ..options()
        };
        let result = stitch_scrolling_frames(&frames, &opts).expect("stitch succeeds");

        assert_eq!(result.image.height(), 16);
        assert_eq!(result.overlap_rows, [0]);
    }

    #[test]
    fn accepts_channel_differences_only_within_tolerance() {
        let frames = [frame(&[10, 20, 30, 40]), frame(&[31, 39, 50, 60])];
        let opts = ScrollStitchOptions {
            row_tolerance: 1,
            ..options()
        };
        let result = stitch_scrolling_frames(&frames, &opts).expect("stitch succeeds");

        assert_eq!(result.image, frame(&[10, 20, 30, 40, 50, 60]));
        assert_eq!(result.overlap_rows, [2]);
        assert_eq!(
            stitch_scrolling_frames(&frames, &options())
                .expect("stitch succeeds")
                .overlap_rows,
            [0]
        );
    }

    #[test]
    fn rejects_empty_frames_mismatched_widths_and_invalid_options() {
        let different_width = CapturedImage::from_rgba(1, 2, vec![0; 8]).expect("valid frame");
        let frames = [frame(&[1, 2]), different_width];
        assert!(
            matches!(stitch_scrolling_frames(&[], &options()), Err(CoreError::CaptureFailed(message)) if message == "no scrolling frames to stitch")
        );
        assert!(
            matches!(stitch_scrolling_frames(&frames, &options()), Err(CoreError::CaptureFailed(message)) if message == "scrolling frame widths differ")
        );

        let opts = ScrollStitchOptions {
            ignore_header_rows: 2,
            ..options()
        };
        assert!(stitch_scrolling_frames(&frames[..1], &opts).is_err());
        let opts = ScrollStitchOptions {
            max_overlap_fraction: f32::NAN,
            ..options()
        };
        assert!(stitch_scrolling_frames(&frames[..1], &opts).is_err());
    }
}
