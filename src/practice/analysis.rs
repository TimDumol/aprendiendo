use std::{io::Cursor, time::Duration};

use serde::{Deserialize, Serialize};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SymphoniaError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub const PROCESSOR_VERSION: &str = "deterministic-delivery-v1";
pub const VAD_MODEL_VERSION: &str = "energy-v1; silero-evaluation-pending";
pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const PROBABILITY_THRESHOLD: f32 = 0.5;
pub const MINIMUM_SPEECH_MS: u32 = 150;
pub const MINIMUM_SILENCE_MS: u32 = 200;
pub const INTERNAL_PAUSE_MS: f64 = 500.0;
pub const LONG_PAUSE_MS: f64 = 2_000.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechInterval {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveryMetrics {
    pub processor_version: String,
    pub model_version: String,
    pub sample_rate_hz: u32,
    pub probability_threshold: f32,
    pub minimum_speech_ms: u32,
    pub minimum_silence_ms: u32,
    pub duration_seconds: f64,
    pub response_span_seconds: Option<f64>,
    pub speech_active_seconds: Option<f64>,
    pub internal_pause_count: Option<u32>,
    pub long_pause_count: Option<u32>,
    pub pause_time_seconds: Option<f64>,
    pub pause_burden: Option<f64>,
    pub pause_frequency_per_minute: Option<f64>,
    pub typical_pause_seconds: Option<f64>,
    pub longest_pause_seconds: Option<f64>,
    pub speech_intervals: Vec<SpeechInterval>,
    pub limitations: Vec<String>,
}

/// Analyze bounded mono PCM. This is intentionally a small deterministic
/// contract while the pinned Silero ONNX spike is evaluated. It uses the same
/// interval and pause definitions as the future VAD stage and marks the model
/// provenance explicitly so the UI cannot present it as acoustic certainty.
pub fn analyze_pcm16le(samples: &[i16], sample_rate_hz: u32) -> DeliveryMetrics {
    let sample_rate_hz = sample_rate_hz.max(1);
    let duration_seconds = samples.len() as f64 / sample_rate_hz as f64;
    let frame_samples = ((sample_rate_hz as usize * 20) / 1_000).max(1);
    let minimum_speech_frames =
        ((MINIMUM_SPEECH_MS as usize * sample_rate_hz as usize) / 1_000 / frame_samples).max(1);
    let minimum_silence_frames =
        ((MINIMUM_SILENCE_MS as usize * sample_rate_hz as usize) / 1_000 / frame_samples).max(1);

    let frames = samples
        .chunks(frame_samples)
        .map(|frame| {
            let peak = frame
                .iter()
                .map(|sample| i32::from(sample.unsigned_abs()))
                .max()
                .unwrap_or_default() as f64
                / i16::MAX as f64;
            let energy = (frame
                .iter()
                .map(|sample| {
                    let normalized = f64::from(*sample) / f64::from(i16::MAX);
                    normalized * normalized
                })
                .sum::<f64>()
                / frame.len().max(1) as f64)
                .sqrt();
            // A bounded energy detector is deliberately conservative. It is
            // not a language-independent definition of hesitation.
            peak >= 0.035 || energy >= 0.012
        })
        .collect::<Vec<_>>();

    let mut smoothed = frames.clone();
    for index in 0..frames.len() {
        let start = index.saturating_sub(1);
        let end = (index + 2).min(frames.len());
        let voiced = frames[start..end].iter().filter(|value| **value).count();
        smoothed[index] = voiced >= 2;
    }

    let mut intervals = Vec::new();
    let mut frame_index = 0;
    while frame_index < smoothed.len() {
        if !smoothed[frame_index] {
            frame_index += 1;
            continue;
        }
        let start = frame_index;
        while frame_index < smoothed.len() && smoothed[frame_index] {
            frame_index += 1;
        }
        if frame_index - start >= minimum_speech_frames {
            intervals.push(SpeechInterval {
                start_seconds: start as f64 * frame_samples as f64 / sample_rate_hz as f64,
                end_seconds: (frame_index * frame_samples).min(samples.len()) as f64
                    / sample_rate_hz as f64,
            });
        }
    }

    // Merge speech intervals separated by less than the configured silence
    // threshold. This avoids making micro-gaps into visible pauses.
    let mut merged: Vec<SpeechInterval> = Vec::new();
    for interval in intervals {
        if let Some(previous) = merged.last_mut() {
            let gap = interval.start_seconds - previous.end_seconds;
            let minimum_silence_seconds =
                minimum_silence_frames as f64 * frame_samples as f64 / sample_rate_hz as f64;
            if gap < minimum_silence_seconds {
                previous.end_seconds = interval.end_seconds;
                continue;
            }
        }
        merged.push(interval);
    }

    let speech_active_seconds = if merged.is_empty() {
        None
    } else {
        Some(
            merged
                .iter()
                .map(|interval| interval.end_seconds - interval.start_seconds)
                .sum(),
        )
    };
    let response_span_seconds = merged
        .first()
        .zip(merged.last())
        .map(|(first, last)| (last.end_seconds - first.start_seconds).max(0.0));
    let gaps = merged
        .windows(2)
        .map(|pair| (pair[1].start_seconds - pair[0].end_seconds).max(0.0))
        .filter(|gap| *gap >= INTERNAL_PAUSE_MS / 1_000.0)
        .collect::<Vec<_>>();
    let long_gaps = gaps
        .iter()
        .copied()
        .filter(|gap| *gap >= LONG_PAUSE_MS / 1_000.0)
        .collect::<Vec<_>>();
    let pause_time_seconds: Option<f64> = if response_span_seconds.is_some() {
        Some(gaps.iter().sum())
    } else {
        None
    };
    let pause_burden: Option<f64> = pause_time_seconds
        .zip(response_span_seconds)
        .filter(|(_, span)| *span > 0.0)
        .map(|(pause, span)| (pause / span).clamp(0.0, 1.0));
    let pause_frequency_per_minute = response_span_seconds
        .filter(|span| *span > 0.0)
        .map(|span| gaps.len() as f64 / (span / 60.0));

    DeliveryMetrics {
        processor_version: PROCESSOR_VERSION.to_owned(),
        model_version: VAD_MODEL_VERSION.to_owned(),
        sample_rate_hz,
        probability_threshold: PROBABILITY_THRESHOLD,
        minimum_speech_ms: MINIMUM_SPEECH_MS,
        minimum_silence_ms: MINIMUM_SILENCE_MS,
        duration_seconds,
        response_span_seconds,
        speech_active_seconds,
        internal_pause_count: response_span_seconds.map(|_| gaps.len() as u32),
        long_pause_count: response_span_seconds.map(|_| long_gaps.len() as u32),
        pause_time_seconds,
        pause_burden,
        pause_frequency_per_minute,
        typical_pause_seconds: if gaps.is_empty() {
            None
        } else {
            Some(median(&gaps))
        },
        longest_pause_seconds: gaps.iter().copied().reduce(f64::max),
        speech_intervals: merged,
        limitations: if speech_active_seconds.is_none() {
            vec!["No reliable speech detected by the bounded energy detector.".to_owned()]
        } else {
            vec![
                "Detected pauses are non-speech intervals; the processor cannot distinguish thinking from breathing.".to_owned(),
                "Quiet speech, noise, music, and other speakers can corrupt the intervals.".to_owned(),
                "The initial release uses a deterministic detector while Silero VAD integration is evaluated.".to_owned(),
            ]
        },
    }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    }
}

/// Decode the small PCM WAV subset used by worker fixtures.
pub fn decode_pcm_wav(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut cursor = 12;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut data = None;
    while cursor + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().ok()?) as usize;
        let end = cursor.checked_add(8)?.checked_add(size)?.min(bytes.len());
        match &bytes[cursor..cursor + 4] {
            b"fmt " if size >= 16 => {
                let format = u16::from_le_bytes(bytes[cursor + 8..cursor + 10].try_into().ok()?);
                if format != 1 {
                    return None;
                }
                channels = u16::from_le_bytes(bytes[cursor + 10..cursor + 12].try_into().ok()?);
                sample_rate = u32::from_le_bytes(bytes[cursor + 12..cursor + 16].try_into().ok()?);
                bits_per_sample =
                    u16::from_le_bytes(bytes[cursor + 22..cursor + 24].try_into().ok()?);
            }
            b"data" => data = Some((cursor + 8, end)),
            _ => {}
        }
        cursor = end + (size % 2);
    }
    if channels == 0 || sample_rate == 0 || bits_per_sample != 16 {
        return None;
    }
    let (start, end) = data?;
    let raw = &bytes[start..end];
    let samples = raw
        .chunks_exact(2 * channels as usize)
        .map(|frame| {
            let total = (0..channels as usize)
                .map(|channel| {
                    i32::from(i16::from_le_bytes([
                        frame[channel * 2],
                        frame[channel * 2 + 1],
                    ]))
                })
                .sum::<i32>();
            (total / i32::from(channels)) as i16
        })
        .collect::<Vec<_>>();
    Some((samples, sample_rate))
}

/// Decode an owned recording once into bounded mono 16 kHz PCM. The archive
/// remains in its original container; this buffer exists only for analysis and
/// is never written to managed storage.
pub fn decode_bounded_audio(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    let (samples, sample_rate) = if let Some(wav) = decode_pcm_wav(bytes) {
        wav
    } else {
        decode_with_symphonia(bytes)?
    };
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }
    if Duration::from_secs(600).as_secs_f64() * (sample_rate as f64) < samples.len() as f64 {
        return None;
    }
    Some((
        resample_linear(&samples, sample_rate, SAMPLE_RATE_HZ),
        SAMPLE_RATE_HZ,
    ))
}

fn decode_with_symphonia(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?.clone();
    let sample_rate = track.codec_params.sample_rate?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;
    let mut mono = Vec::new();
    let max_samples = sample_rate as usize * 600;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => return None,
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = decoder.decode(&packet).ok()?;
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        mono.extend(buffer.samples().chunks(channels).map(|frame| {
            let total = frame.iter().map(|sample| i32::from(*sample)).sum::<i32>();
            (total / frame.len().max(1) as i32) as i16
        }));
        if mono.len() > max_samples {
            return None;
        }
    }
    Some((mono, sample_rate))
}

fn resample_linear(samples: &[i16], source_rate: u32, target_rate: u32) -> Vec<i16> {
    if source_rate == target_rate {
        return samples.to_vec();
    }
    let output_len = ((samples.len() as u64 * target_rate as u64) / source_rate as u64) as usize;
    (0..output_len)
        .map(|index| {
            let source_position = index as f64 * source_rate as f64 / target_rate as f64;
            let lower = source_position.floor() as usize;
            let upper = (lower + 1).min(samples.len().saturating_sub(1));
            let fraction = source_position - lower as f64;
            let value = samples[lower] as f64 * (1.0 - fraction) + samples[upper] as f64 * fraction;
            value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech_with_gap() -> Vec<i16> {
        let mut samples = Vec::new();
        samples.extend(std::iter::repeat_n(8_000, SAMPLE_RATE_HZ as usize));
        samples.extend(std::iter::repeat_n(0, SAMPLE_RATE_HZ as usize));
        samples.extend(std::iter::repeat_n(8_000, SAMPLE_RATE_HZ as usize));
        samples
    }

    #[test]
    fn reports_pause_definitions_and_denominators() {
        let result = analyze_pcm16le(&speech_with_gap(), SAMPLE_RATE_HZ);
        assert_eq!(result.internal_pause_count, Some(1));
        assert_eq!(result.long_pause_count, Some(0));
        assert!(result.pause_time_seconds.unwrap() >= 0.9);
        assert!(result.response_span_seconds.unwrap() > 1.9);
        assert_eq!(result.processor_version, PROCESSOR_VERSION);
    }

    #[test]
    fn silent_audio_is_unknown_rather_than_zero_pauses() {
        let result = analyze_pcm16le(&vec![0; SAMPLE_RATE_HZ as usize], SAMPLE_RATE_HZ);
        assert_eq!(result.response_span_seconds, None);
        assert_eq!(result.internal_pause_count, None);
        assert!(result.limitations[0].contains("No reliable speech"));
    }

    #[test]
    fn rejects_compressed_wav_formats() {
        let mut bytes = vec![0u8; 44];
        bytes[0..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WAVE");
        bytes[12..16].copy_from_slice(b"fmt ");
        bytes[16..20].copy_from_slice(&16u32.to_le_bytes());
        bytes[20..22].copy_from_slice(&3u16.to_le_bytes());
        bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
        bytes[24..28].copy_from_slice(&16_000u32.to_le_bytes());
        assert!(decode_pcm_wav(&bytes).is_none());
    }
}
