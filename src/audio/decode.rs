use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use super::AudioClip;

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "mp3", "ogg", "oga", "flac", "m4a", "mp4", "aac", "caf",
    "mkv", "webm",
];

pub fn load(path: &Path) -> Result<AudioClip> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint_for(path),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| anyhow!("unrecognised audio container: {error}"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("no audio track"))?;
    let track_id = track.id;
    let params = match &track.codec_params {
        Some(CodecParameters::Audio(params)) => params.clone(),
        _ => bail!("track has no audio codec parameters"),
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|error| anyhow!("unsupported codec: {error}"))?;

    let mut clip: Option<AudioClip> = None;
    let mut scratch: Vec<Vec<f32>> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(error) => return Err(anyhow!("reading packet: {error}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(anyhow!("decoding: {error}")),
        };
        let spec = decoded.spec();
        let target =
            clip.get_or_insert_with(|| AudioClip::new(spec.rate(), spec.channels().count()));
        let frames = decoded.frames();
        scratch.resize(decoded.num_planes(), Vec::new());
        for plane in &mut scratch {
            plane.resize(frames, 0.0);
        }
        decoded.copy_to_slice_planar(&mut scratch);
        for (channel, plane) in target.channels.iter_mut().zip(&scratch) {
            channel.extend_from_slice(plane);
        }
    }
    clip.ok_or_else(|| anyhow!("file contained no decodable audio"))
}

fn hint_for(path: &Path) -> Hint {
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }
    hint
}
