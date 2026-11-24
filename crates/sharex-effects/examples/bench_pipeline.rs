use sharex_core::{CapturedImage, ImageFormat};
use sharex_effects::builtin_presets;
use std::hint::black_box;
use std::time::Instant;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const ITERATIONS: usize = 3;

fn generate_synthetic_1080p_frame() -> CapturedImage {
    let pixel_count = (WIDTH as usize) * (HEIGHT as usize);
    let mut rgba = Vec::with_capacity(pixel_count * 4);

    for y in 0..HEIGHT {
        let ny = (y * 255 / HEIGHT) as u8;
        let is_grid_row = y % 64 == 0;
        for x in 0..WIDTH {
            let nx = (x * 255 / WIDTH) as u8;
            let is_grid_col = x % 64 == 0;
            let checker = ((x / 32) ^ (y / 32)) & 1;

            let (r, g, b) = if is_grid_row || is_grid_col {
                (230, 236, 245)
            } else if (240..1680).contains(&x) && (160..920).contains(&y) {
                let panel_boost = if checker == 0 { 12 } else { 0 };
                (
                    32u8.saturating_add(panel_boost),
                    40u8.saturating_add(ny / 6),
                    58u8.saturating_add(nx / 5),
                )
            } else {
                (
                    nx,
                    ny,
                    ((x.wrapping_mul(3) ^ y.wrapping_mul(5)) & 0xff) as u8,
                )
            };

            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }

    CapturedImage::from_rgba(WIDTH, HEIGHT, rgba).expect("valid synthetic 1080p frame")
}

fn main() {
    let presets = builtin_presets();
    let clean_screenshot = presets
        .into_iter()
        .find(|preset| preset.name == "Clean Screenshot")
        .expect("Clean Screenshot builtin preset must exist");

    let gen_start = Instant::now();
    let frame = generate_synthetic_1080p_frame();
    let gen_ms = gen_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "ShareX-RS 1080p Pipeline Benchmark ({WIDTH}x{HEIGHT} RGBA, {ITERATIONS} iterations, preset: {})",
        clean_screenshot.name
    );
    println!("frame_generation: {gen_ms:.2} ms");
    println!();

    let mut raw_png_total_ms = 0.0;
    let mut effects_total_ms = 0.0;
    let mut processed_png_total_ms = 0.0;
    let mut last_output_dims = (0u32, 0u32);
    let mut last_raw_png_bytes = 0usize;
    let mut last_processed_png_bytes = 0usize;

    for iter in 1..=ITERATIONS {
        let raw_png_start = Instant::now();
        let raw_png = black_box(
            frame
                .encode(ImageFormat::Png)
                .expect("encode raw 1080p PNG"),
        );
        let raw_png_ms = raw_png_start.elapsed().as_secs_f64() * 1000.0;

        let effects_start = Instant::now();
        let processed = black_box(
            clean_screenshot
                .apply(&frame)
                .expect("apply Clean Screenshot preset"),
        );
        let effects_ms = effects_start.elapsed().as_secs_f64() * 1000.0;

        let processed_png_start = Instant::now();
        let processed_png = black_box(
            processed
                .encode(ImageFormat::Png)
                .expect("encode processed PNG"),
        );
        let processed_png_ms = processed_png_start.elapsed().as_secs_f64() * 1000.0;

        let pipeline_ms = effects_ms + processed_png_ms;

        raw_png_total_ms += raw_png_ms;
        effects_total_ms += effects_ms;
        processed_png_total_ms += processed_png_ms;
        last_output_dims = (processed.width(), processed.height());
        last_raw_png_bytes = raw_png.len();
        last_processed_png_bytes = processed_png.len();

        println!(
            "iter {iter}/{ITERATIONS} | raw_png_1080p: {raw_png_ms:7.2} ms | effects_chain: {effects_ms:7.2} ms | processed_png: {processed_png_ms:7.2} ms | effects+png: {pipeline_ms:7.2} ms"
        );
    }

    let count = ITERATIONS as f64;
    let avg_raw_png_ms = raw_png_total_ms / count;
    let avg_effects_ms = effects_total_ms / count;
    let avg_processed_png_ms = processed_png_total_ms / count;
    let avg_pipeline_ms = (effects_total_ms + processed_png_total_ms) / count;

    println!();
    println!(
        "summary (avg ms/frame) | raw_png_1080p: {avg_raw_png_ms:.2} ms | effects_chain: {avg_effects_ms:.2} ms | processed_png: {avg_processed_png_ms:.2} ms | effects+png: {avg_pipeline_ms:.2} ms"
    );
    println!(
        "output | processed_dims: {}x{} | raw_png_bytes: {} | processed_png_bytes: {}",
        last_output_dims.0, last_output_dims.1, last_raw_png_bytes, last_processed_png_bytes
    );
}
