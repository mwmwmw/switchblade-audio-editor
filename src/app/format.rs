use crate::audio::db::SILENCE_DB;

const MILLIS_PER_SECOND: f64 = 1000.0;

pub fn time(frames: usize, sample_rate: u32) -> String {
    if sample_rate == 0 {
        return "0:00.000".into();
    }
    seconds(frames as f64 / sample_rate as f64)
}

pub fn seconds(total: f64) -> String {
    let total_millis = (total * MILLIS_PER_SECOND).round() as u64;
    let millis = total_millis % 1000;
    let whole = total_millis / 1000;
    let (hours, minutes, secs) = (whole / 3600, (whole / 60) % 60, whole % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}.{millis:03}")
    } else {
        format!("{minutes}:{secs:02}.{millis:03}")
    }
}

pub fn db(value: f32) -> String {
    if value <= SILENCE_DB + 1.0 {
        "-∞".into()
    } else {
        format!("{value:+.1}")
    }
}

pub fn lufs(value: f64) -> String {
    if value <= -70.0 {
        "-∞".into()
    } else {
        format!("{value:.1}")
    }
}

pub fn sample_rate(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", rate as f64 / 1000.0)
    }
}

pub fn channels(count: usize) -> &'static str {
    match count {
        1 => "mono",
        2 => "stereo",
        _ => "multichannel",
    }
}
