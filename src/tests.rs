use std::f32::consts::TAU;
use std::path::PathBuf;

use crate::analysis::anomalies::{scan, AnomalyKind, AnomalySettings};
use crate::analysis::loudness::measure;
use crate::analysis::spectrum::{average_band_spectrum, REFERENCE_BAND_INDEX};
use crate::analysis::tonal::{equal_loudness_contour, TonalBalance};
use crate::audio::decode::load;
use crate::audio::encode::{save, BitDepth, ExportFormat, SaveOptions};
use crate::audio::fade::{apply_fade, FadeCurve, FadeDirection};
use crate::audio::normalize::normalize_peak;
use crate::audio::resample::{resample, ResampleQuality};
use crate::audio::AudioClip;

const TEST_RATE: u32 = 44_100;
const TONE_HZ: f32 = 1000.0;
/// Export dithers, so a roundtrip differs by up to 1.5 LSB: the TPDF noise is under 1 LSB
/// and rounding adds another half. The tolerances allow 2 LSB to leave a little margin.
const INT16_TOLERANCE: f32 = 2.0 / 32_768.0;
const INT24_TOLERANCE: f32 = 2.0 / 8_388_608.0;

fn sine_clip(rate: u32, seconds: f32, amplitude: f32) -> AudioClip {
    let frames = (rate as f32 * seconds) as usize;
    let tone: Vec<f32> = (0..frames)
        .map(|i| amplitude * (TAU * TONE_HZ * i as f32 / rate as f32).sin())
        .collect();
    AudioClip {
        sample_rate: rate,
        channels: vec![tone.clone(), tone],
    }
}

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("switchblade-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn max_abs_difference(a: &AudioClip, b: &AudioClip) -> f32 {
    a.channels
        .iter()
        .zip(&b.channels)
        .flat_map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p - q).abs()))
        .fold(0.0, f32::max)
}

fn roundtrip(format: ExportFormat, depth: BitDepth, tolerance: f32) {
    let clip = sine_clip(TEST_RATE, 0.25, 0.8);
    let path = temp_path(&format!(
        "roundtrip_{}_{}.{}",
        format.label(),
        depth.bits(),
        format.extension()
    ));
    save(&path, &clip, format, depth, &SaveOptions::default()).unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.sample_rate, TEST_RATE);
    assert_eq!(loaded.channel_count(), 2);
    assert_eq!(loaded.frames(), clip.frames());
    assert!(
        max_abs_difference(&clip, &loaded) < tolerance,
        "{format:?} {depth:?}"
    );
}

#[test]
fn wav_roundtrips() {
    roundtrip(ExportFormat::Wav, BitDepth::Int16, INT16_TOLERANCE);
    roundtrip(ExportFormat::Wav, BitDepth::Int24, INT24_TOLERANCE);
    roundtrip(ExportFormat::Wav, BitDepth::Float32, 1e-7);
}

#[test]
fn aiff_roundtrips() {
    roundtrip(ExportFormat::Aiff, BitDepth::Int16, INT16_TOLERANCE);
    roundtrip(ExportFormat::Aiff, BitDepth::Int24, INT24_TOLERANCE);
    roundtrip(ExportFormat::Aiff, BitDepth::Float32, 1e-7);
}

#[test]
fn flac_roundtrips() {
    roundtrip(ExportFormat::Flac, BitDepth::Int16, INT16_TOLERANCE);
    roundtrip(ExportFormat::Flac, BitDepth::Int24, INT24_TOLERANCE);
}

#[test]
fn interleave_roundtrip_and_editing() {
    let mut clip = AudioClip::from_interleaved(8, 2, &[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
    assert_eq!(clip.to_interleaved(), vec![1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
    clip.remove(&(1..2));
    assert_eq!(clip.channels[0], vec![1.0, 3.0]);
    let mono = AudioClip {
        sample_rate: 8,
        channels: vec![vec![9.0]],
    };
    clip.insert(1, &mono);
    assert_eq!(clip.channels[0], vec![1.0, 9.0, 3.0]);
    assert_eq!(clip.channels[1], vec![-1.0, 9.0, -3.0]);
}

#[test]
fn resampling_preserves_duration_and_level() {
    let clip = sine_clip(44_100, 0.5, 0.5);
    for quality in ResampleQuality::ALL {
        let resampled = resample(&clip, 48_000, quality).unwrap();
        assert_eq!(resampled.sample_rate, 48_000);
        let expected = (clip.frames() as f64 * 48_000.0 / 44_100.0).round() as usize;
        assert!(
            (resampled.frames() as i64 - expected as i64).abs() <= 2,
            "{quality:?}: {}",
            resampled.frames()
        );
        let peak = resampled.peak(&(1000..resampled.frames() - 1000));
        assert!((peak - 0.5).abs() < 0.01, "{quality:?}: peak {peak}");
    }
}

#[test]
fn normalize_hits_target() {
    let mut clip = sine_clip(TEST_RATE, 0.1, 0.25);
    let range = clip.full_range();
    let gain = normalize_peak(&mut clip, &range, -6.0).unwrap();
    let peak = clip.peak(&clip.full_range());
    assert!((20.0 * peak.log10() + 6.0).abs() < 0.01);
    assert!((gain - 6.02).abs() < 0.1);
}

#[test]
fn fades_start_silent_and_end_at_unity() {
    for curve in FadeCurve::ALL {
        assert!(curve.gain_at(0.0).abs() < 1e-6, "{curve:?}");
        assert!((curve.gain_at(1.0) - 1.0).abs() < 1e-6, "{curve:?}");
        assert!(
            curve.gain_at(0.5) > 0.0 && curve.gain_at(0.5) < 1.0,
            "{curve:?}"
        );
    }
    let mut clip = AudioClip {
        sample_rate: 10,
        channels: vec![vec![1.0; 11]],
    };
    let range = clip.full_range();
    apply_fade(&mut clip, &range, FadeCurve::Linear, FadeDirection::Out);
    assert_eq!(clip.channels[0][0], 1.0);
    assert!((clip.channels[0][5] - 0.5).abs() < 1e-6);
    assert_eq!(clip.channels[0][10], 0.0);
}

#[test]
fn anomaly_scan_finds_each_kind() {
    let mut samples = vec![0.1_f32; 1000];
    samples[100..104].fill(1.0);
    samples[300..400].fill(0.0);
    samples[700] = 0.9;
    let clip = AudioClip {
        sample_rate: TEST_RATE,
        channels: vec![samples],
    };
    let report = scan(&clip, &AnomalySettings::default());
    assert_eq!(report.count(AnomalyKind::Clip), 1);
    assert_eq!(report.count(AnomalyKind::ZeroRun), 1);
    // Both edges of the clipped run and both edges of the spike are jumps.
    assert_eq!(report.count(AnomalyKind::Discontinuity), 4);
    let clip_run = report
        .anomalies
        .iter()
        .find(|a| a.kind == AnomalyKind::Clip)
        .unwrap();
    assert_eq!(clip_run.range, 100..104);
}

#[test]
fn full_scale_stereo_sine_measures_about_zero_lufs() {
    // BS.1770: a 0 dBFS 1 kHz sine in one channel reads -3.01 LKFS, so in both channels it reads 0.
    let report = measure(&sine_clip(48_000, 2.0, 1.0)).unwrap();
    assert!(
        report.integrated_lufs.abs() < 0.5,
        "{}",
        report.integrated_lufs
    );
    assert!(
        report.true_peak_dbtp.abs() < 0.3,
        "{}",
        report.true_peak_dbtp
    );
}

#[test]
fn spectrum_peaks_in_the_one_kilohertz_band() {
    let spectrum = average_band_spectrum(&sine_clip(48_000, 1.0, 0.5));
    let loudest = spectrum
        .levels_db
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    assert_eq!(loudest, REFERENCE_BAND_INDEX);
    let balance = TonalBalance::against_reference(&spectrum, &spectrum);
    assert!(balance.deviation_db.iter().all(|d| d.abs() < 1e-3));
    for balance in [
        TonalBalance::against_equal_loudness(&spectrum, 80.0),
        TonalBalance::against_pink(&spectrum),
    ] {
        let mean: f32 =
            balance.deviation_db.iter().sum::<f32>() / balance.deviation_db.len() as f32;
        assert!(
            mean.abs() < 1e-3,
            "deviations must be anchored on their mean"
        );
        let loudest = balance
            .deviation_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(loudest, REFERENCE_BAND_INDEX);
    }
}

#[test]
fn iso226_contour_matches_published_values() {
    let contour = equal_loudness_contour(40.0);
    assert!(
        (contour[REFERENCE_BAND_INDEX] - 40.0).abs() < 0.3,
        "{}",
        contour[REFERENCE_BAND_INDEX]
    );
    assert!(
        (contour[0] - 99.9).abs() < 1.0,
        "20 Hz at 40 phon: {}",
        contour[0]
    );
    assert!(
        contour[23] < contour[REFERENCE_BAND_INDEX],
        "4 kHz should need less SPL than 1 kHz"
    );
}

/// Exercises the compressed decoders when ffmpeg is available to produce the fixtures.
#[test]
fn decodes_formats_transcoded_by_ffmpeg() {
    let source = temp_path("transcode_source.wav");
    let clip = sine_clip(TEST_RATE, 0.5, 0.5);
    save(
        &source,
        &clip,
        ExportFormat::Wav,
        BitDepth::Int16,
        &SaveOptions::default(),
    )
    .unwrap();
    for extension in ["mp3", "ogg", "aiff", "flac", "m4a"] {
        let target = temp_path(&format!("transcoded.{extension}"));
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&source)
            .arg(&target)
            .status();
        let Ok(status) = status else {
            eprintln!("ffmpeg not available; skipping transcode fixtures");
            return;
        };
        assert!(status.success(), "ffmpeg failed for {extension}");
        let decoded = load(&target).unwrap_or_else(|e| panic!("{extension}: {e:#}"));
        assert_eq!(decoded.sample_rate, TEST_RATE, "{extension}");
        assert_eq!(decoded.channel_count(), 2, "{extension}");
        let tolerance_frames = if extension == "aiff" || extension == "flac" {
            0
        } else {
            2500
        };
        let difference = (decoded.frames() as i64 - clip.frames() as i64).unsigned_abs() as usize;
        assert!(
            difference <= tolerance_frames,
            "{extension}: {} frames vs {}",
            decoded.frames(),
            clip.frames()
        );
        let peak = decoded.peak(&decoded.full_range());
        assert!((peak - 0.5).abs() < 0.08, "{extension}: peak {peak}");
    }
}

/// Plays through the real default output device; skipped when the machine has none.
#[test]
fn engine_plays_through_default_output() {
    use crate::engine::{Engine, EngineCommand, EngineEvent};
    use std::sync::atomic::Ordering;
    let engine = Engine::start();
    let clip = std::sync::Arc::new(sine_clip(TEST_RATE, 2.0, 0.2));
    engine.send(EngineCommand::Play {
        clip,
        start_frame: 0,
        loop_range: None,
    });
    std::thread::sleep(std::time::Duration::from_millis(600));
    let events = engine.poll_events();
    let mut opened = false;
    for event in &events {
        match event {
            EngineEvent::StreamOpened { .. } => opened = true,
            EngineEvent::Error(message) if message.contains("no output device") => {
                eprintln!("no output device; skipping");
                return;
            }
            EngineEvent::Error(message) if message.contains("underrun") => eprintln!("{message}"),
            EngineEvent::Error(message) => panic!("engine error: {message}"),
            _ => {}
        }
    }
    assert!(opened, "stream never opened: {} events", events.len());
    assert!(engine.shared.is_playing());
    assert!(engine.shared.play_position.load(Ordering::Relaxed) > TEST_RATE as usize / 10);
    let meters = *engine.shared.meters.lock();
    assert!(
        meters.peak_dbfs[0] > -20.0,
        "meter did not register audio: {}",
        meters.peak_dbfs[0]
    );
    engine.send(EngineCommand::Stop);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!engine.shared.is_playing());
}

#[test]
fn export_dither_breaks_up_a_constant_rounding_error() {
    // Every sample sits exactly half a 16-bit code above zero. Undithered, they all round
    // the same way and the output is a constant; dither has to split them between the two
    // neighbouring codes, which is the whole point of adding it.
    let half_lsb = 0.5 / 32_768.0;
    let clip = AudioClip {
        sample_rate: TEST_RATE,
        channels: vec![vec![half_lsb; 4096]],
    };
    let path = temp_path("dither_half_lsb.wav");
    save(
        &path,
        &clip,
        ExportFormat::Wav,
        BitDepth::Int16,
        &SaveOptions::default(),
    )
    .unwrap();
    let loaded = load(&path).unwrap();
    let codes: Vec<i32> = loaded.channels[0]
        .iter()
        .map(|s| (s * 32_768.0).round() as i32)
        .collect();
    assert!(
        codes.contains(&0) && codes.contains(&1),
        "dither should straddle both codes, saw {:?}..{:?}",
        codes.iter().min(),
        codes.iter().max()
    );
    // It must stay centred: a biased dither would shift the signal's level.
    let mean = codes.iter().map(|c| *c as f64).sum::<f64>() / codes.len() as f64;
    assert!((mean - 0.5).abs() < 0.05, "dither is biased: mean code {mean}");
}

/// Short decaying noise bursts at a fixed interval: a click track with clear attacks.
fn click_track(rate: u32, bpm: f64, beats: usize) -> AudioClip {
    let interval = (60.0 / bpm * rate as f64) as usize;
    let mut samples = vec![0.0_f32; interval * beats];
    let burst = rate as usize / 100; // 10 ms
    let mut noise: u32 = 0x1234_5678;
    for beat in 0..beats {
        let start = beat * interval;
        for offset in 0..burst {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (noise >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
            let decay = 1.0 - offset as f32 / burst as f32;
            samples[start + offset] = white * decay * decay * 0.8;
        }
    }
    AudioClip {
        sample_rate: rate,
        channels: vec![samples.clone(), samples],
    }
}

#[test]
fn beat_detection_finds_the_tempo_and_every_hit() {
    use crate::analysis::beats::{detect, BeatSettings};
    let bpm = 120.0;
    let beats = 16;
    let clip = click_track(TEST_RATE, bpm, beats);
    let report = detect(&clip, &BeatSettings::default());

    let detected = report.bpm.expect("a click track must yield a tempo");
    assert!(
        (detected as f64 - bpm).abs() < 2.0,
        "expected about {bpm} BPM, got {detected}"
    );
    assert_eq!(
        report.beats.len(),
        beats,
        "every click should be picked exactly once"
    );

    // Hits are timed from the analysis window that first saw them, so they land within a
    // hop or so of the attack. 10 ms is the useful bound for snapping the cursor to one.
    let interval = (60.0 / bpm * TEST_RATE as f64) as usize;
    let tolerance = TEST_RATE as usize / 100;
    for (index, beat) in report.beats.iter().enumerate() {
        let expected = index * interval;
        assert!(
            beat.frame.abs_diff(expected) <= tolerance,
            "hit {index} at frame {} should be within {tolerance} of {expected}",
            beat.frame
        );
    }
}

#[test]
fn beat_detection_separates_a_real_attack_from_the_noise_floor() {
    use crate::analysis::beats::{detect, BeatSettings};
    // A sine that starts abruptly from silence has exactly one attack — its own onset. The
    // envelope is normalised against its own 95th percentile, so the steady tone that follows
    // still throws up weak peaks; the onset has to stand clearly above all of them.
    let report = detect(&sine_clip(TEST_RATE, 2.0, 0.5), &BeatSettings::default());
    let first = report.beats.first().expect("the onset should be found");
    assert!(
        first.frame <= TEST_RATE as usize / 100,
        "the onset belongs at the start, not frame {}",
        first.frame
    );
    let loudest_after = report.beats[1..]
        .iter()
        .map(|beat| beat.strength)
        .fold(0.0_f32, f32::max);
    assert!(
        first.strength > loudest_after * 2.0,
        "onset ({:.2}) should tower over the noise floor ({loudest_after:.2})",
        first.strength
    );
}

#[test]
fn beat_detection_reports_every_click_at_full_strength() {
    use crate::analysis::beats::{detect, BeatSettings};
    // Identical clicks should read as identical hits: nothing in the chain may favour the
    // start or end of the file, which is what a mis-timed analysis window would show up as.
    let report = detect(&click_track(TEST_RATE, 140.0, 12), &BeatSettings::default());
    assert_eq!(report.beats.len(), 12);
    let strengths: Vec<f32> = report.beats.iter().map(|beat| beat.strength).collect();
    let weakest = strengths.iter().copied().fold(f32::MAX, f32::min);
    let strongest = strengths.iter().copied().fold(0.0_f32, f32::max);
    assert!(
        weakest > strongest * 0.9,
        "identical clicks should score alike, got {weakest:.2}..{strongest:.2}"
    );
}

#[test]
fn loop_bounds_published_for_the_audio_thread() {
    use crate::engine::SharedState;
    let shared = SharedState::default();
    assert_eq!(shared.loop_range(), None);
    shared.set_loop(Some(100..200));
    assert_eq!(shared.loop_range(), Some(100..200));
    // Resizing the loop replaces it outright, so the audio thread sees the new bounds.
    shared.set_loop(Some(100..150));
    assert_eq!(shared.loop_range(), Some(100..150));
    // Empty and inverted ranges clear the loop rather than trapping playback in zero frames.
    shared.set_loop(Some(150..150));
    assert_eq!(shared.loop_range(), None);
    shared.set_loop(Some(200..100));
    assert_eq!(shared.loop_range(), None);
    shared.set_loop(Some(1..2));
    shared.set_loop(None);
    assert_eq!(shared.loop_range(), None);
}

#[test]
fn new_documents_never_reuse_a_version() {
    use crate::document::Document;
    let first = Document::default();
    let mut second = Document::from_clip(sine_clip(TEST_RATE, 0.1, 0.5), None);
    assert_ne!(first.version, second.version);
    let before = second.version;
    second.commit("edit", sine_clip(TEST_RATE, 0.2, 0.5));
    assert_ne!(before, second.version);
    assert_ne!(first.version, second.version);
}

#[test]
fn analysis_cache_roundtrips_through_sidecar() {
    use crate::analysis::anomalies::AnomalySettings;
    use crate::analysis::report::analyze;
    use crate::cache;
    let path = temp_path("cached_source.wav");
    let mut clip = sine_clip(TEST_RATE, 0.5, 0.9);
    clip.channels[0][1000..1010].fill(1.0);
    save(
        &path,
        &clip,
        ExportFormat::Wav,
        BitDepth::Float32,
        &SaveOptions::default(),
    )
    .unwrap();
    let report = analyze(&clip, &AnomalySettings::default());
    let sidecar = cache::store(&path, &report).unwrap();
    assert!(sidecar.exists());
    let loaded = cache::load(&path).expect("cache should load for an unchanged file");
    assert_eq!(loaded.peaks, report.peaks);
    assert_eq!(
        loaded.anomalies.anomalies.len(),
        report.anomalies.anomalies.len()
    );
    assert_eq!(loaded.spectrum.levels_db, report.spectrum.levels_db);
    assert!(loaded.is_current(&clip, &AnomalySettings::default()));
    let other_settings = AnomalySettings {
        clip_min_run: 40,
        ..AnomalySettings::default()
    };
    assert!(!loaded.is_current(&clip, &other_settings));
    save(
        &path,
        &sine_clip(TEST_RATE, 0.6, 0.9),
        ExportFormat::Wav,
        BitDepth::Float32,
        &SaveOptions::default(),
    )
    .unwrap();
    assert!(
        cache::load(&path).is_none(),
        "cache must be rejected once the file changes"
    );
}
