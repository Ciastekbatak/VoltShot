use std::f32::consts::TAU;

const SAMPLE_RATE: u32 = 22_050;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;
const DURATION_MS: u32 = 45;
const WAV_HEADER_BYTES: usize = 44;

/// Generates a crisp 45ms synthesized camera shutter click as a 16-bit mono 22050Hz RIFF/WAVE buffer.
pub fn generate_shutter_wav() -> Vec<u8> {
    let sample_count = ((SAMPLE_RATE * DURATION_MS) / 1000) as usize;
    let bytes_per_sample = usize::from(BITS_PER_SAMPLE / 8);
    let data_bytes = sample_count * usize::from(CHANNELS) * bytes_per_sample;
    let riff_chunk_size = (36 + data_bytes) as u32;
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);

    let mut wav = Vec::with_capacity(WAV_HEADER_BYTES + data_bytes);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_chunk_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_bytes as u32).to_le_bytes());

    let duration_secs = sample_count as f32 / SAMPLE_RATE as f32;
    for i in 0..sample_count {
        let t = i as f32 / SAMPLE_RATE as f32;
        let attack = (t / 0.0012).min(1.0);
        let release = ((duration_secs - t) / 0.004).clamp(0.0, 1.0);

        let click_env = (-t * 280.0).exp();
        let click_wave = (TAU * 1_950.0 * t).sin() * 0.55 + (TAU * 3_600.0 * t).sin() * 0.45;

        let chime_t = (t - 0.008).max(0.0);
        let chime_gate = if t >= 0.008 { 1.0 } else { 0.0 };
        let chime_env = chime_gate * (-chime_t * 95.0).exp();
        let chime_wave =
            (TAU * 1_320.0 * chime_t).sin() * 0.6 + (TAU * 2_640.0 * chime_t).sin() * 0.4;

        let mixed = (click_wave * click_env * 0.65 + chime_wave * chime_env * 0.35)
            * attack
            * release
            * 0.55;
        let sample = (mixed.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }

    wav
}

/// Plays the synthesized capture shutter sound asynchronously without blocking the UI thread.
pub fn play_capture_sound() {
    imp::play_capture_sound();
}

#[cfg(target_os = "windows")]
mod imp {
    use super::generate_shutter_wav;
    use std::ptr;
    use std::sync::OnceLock;
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    static SHUTTER_WAV: OnceLock<Vec<u8>> = OnceLock::new();

    pub fn play_capture_sound() {
        let wav = SHUTTER_WAV.get_or_init(generate_shutter_wav);
        unsafe {
            PlaySoundW(
                wav.as_ptr().cast::<u16>(),
                ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub fn play_capture_sound() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_shutter_wav_has_valid_riff_wave_structure_and_nonzero_samples() {
        let wav = generate_shutter_wav();
        assert!(wav.len() > WAV_HEADER_BYTES);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");

        let riff_size = u32::from_le_bytes(wav[4..8].try_into().expect("4 bytes")) as usize;
        let fmt_chunk_size = u32::from_le_bytes(wav[16..20].try_into().expect("4 bytes"));
        let audio_format = u16::from_le_bytes(wav[20..22].try_into().expect("2 bytes"));
        let channels = u16::from_le_bytes(wav[22..24].try_into().expect("2 bytes"));
        let sample_rate = u32::from_le_bytes(wav[24..28].try_into().expect("4 bytes"));
        let byte_rate = u32::from_le_bytes(wav[28..32].try_into().expect("4 bytes"));
        let block_align = u16::from_le_bytes(wav[32..34].try_into().expect("2 bytes"));
        let bits_per_sample = u16::from_le_bytes(wav[34..36].try_into().expect("2 bytes"));
        let data_size = u32::from_le_bytes(wav[40..44].try_into().expect("4 bytes")) as usize;

        assert_eq!(riff_size + 8, wav.len());
        assert_eq!(data_size + WAV_HEADER_BYTES, wav.len());
        assert_eq!(fmt_chunk_size, 16);
        assert_eq!(audio_format, 1);
        assert_eq!(channels, 1);
        assert_eq!(sample_rate, 22_050);
        assert_eq!(byte_rate, 44_100);
        assert_eq!(block_align, 2);
        assert_eq!(bits_per_sample, 16);

        let samples: Vec<i16> = wav[WAV_HEADER_BYTES..]
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(samples.len(), ((SAMPLE_RATE * DURATION_MS) / 1000) as usize);
        let peak = samples
            .iter()
            .map(|&s| i32::from(s).abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 1_000, "expected audible shutter peak, got {peak}");
    }
}
