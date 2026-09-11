# Switchblade

A small, fast audio editor written in Rust. It opens in well under a second, shows one file
at a time, and puts clipping, dropouts and bad edits front and centre.

## Features (v1)

- **Open** WAV, AIFF/AIFC, FLAC, MP3, OGG Vorbis, AAC/M4A, CAF, MKV/WebM audio (via symphonia).
- **Save** WAV, AIFF/AIFC and FLAC at 16-bit, 24-bit or 32-bit float (FLAC: 16/24), with
  TPDF dither on every fixed-point export.
- **Edit**: cut, copy, paste, delete, trim, silence, with snapshot undo/redo.
- **Resample** to any rate with a 256-tap windowed-sinc resampler (or a fast FFT mode).
- **Normalize** peak to 0, −0.03, −3, −6, −12, −18 dBFS or a custom level.
- **Fades** in/out over the selection or a duration: linear, equal-power, exponential,
  logarithmic, S-curve.
- **Repair** the discontinuities the scan flags, blending a short window across each jump,
  and **remove DC offset** per channel.
- **Seamless loops**: crossfade the run-up to the loop over its tail with a Hann pair, and
  write the loop points into the saved file (WAV `smpl`, AIFF `MARK`/`INST`).
- **Beat detection**: spectral-flux onsets and an autocorrelation tempo estimate, ported from
  the m8-groove extractor. Detected hits draw on the ruler and the cursor and selection edges
  snap to them (View → Snap to beats); the tempo shows in the status bar.
- **Plugin stack**: CLAP and VST2 plugins run live during playback and can be rendered
  into the file. VST3 bundles are discovered and listed but not yet hosted.
- **Tonal balance** per third-octave band, three ways: perceived loudness after the
  ISO 226:2003 equal-loudness contour (the modern Fletcher–Munson curves) at a chosen
  listening level, physical level against pink noise, or against a reference track.
  With a selection, the chart and the file loudness readouts re-run on that region.
- **Record** from any input device on any driver cpal exposes: CoreAudio, WASAPI, ALSA,
  JACK (`--features jack`) and ASIO (`--features asio`, Windows, needs the ASIO SDK).
- **Highlights**: clipped runs (red), zero runs / dropouts (blue) and sample
  discontinuities (orange), with thresholds you can tune (View → Highlight settings).
- **Meters**: per-channel dBFS peak with hold, VU with selectable 0 VU reference
  (−8 to −20 dBFS), LUFS momentary / short-term / integrated, true peak, loudness range.

## Build and run

```sh
cargo run --release -- path/to/file.wav
```

Optional backends: `cargo build --release --features jack` or `--features asio`.

### Windows

Building on Windows needs the MSVC toolchain (Visual Studio Build Tools, C++ workload);
everything else is system-provided — WASAPI for audio, the native file dialog, the OS
OpenGL driver. `--features asio` needs the ASIO SDK, `CPAL_ASIO_DIR` and LLVM for bindgen,
and only builds on Windows itself.

From WSL, `scripts/win-run.sh` cross-builds, copies the exe to `%USERPROFILE%\Switchblade`
and starts it on the Windows side:

```sh
scripts/win-run.sh path/to/file.wav     # build, stage, launch
scripts/win-run.sh --debug --wait       # console build, output relayed to this terminal
scripts/win-run.sh -n                   # stage only
```

It needs `rustup target add x86_64-pc-windows-gnu` and `mingw-w64`; the resulting exe links
only against system DLLs, so it also runs on a machine with no toolchain. `SWITCHBLADE_WIN_DIR`
overrides the staging directory and `RUST_LOG` is forwarded to the app (default `warn`).

## Controls

| Action | Keys |
| --- | --- |
| Play / pause, stop | Space, Esc |
| Record, loop selection | R, L |
| Select by dragging, select all | drag, ⌘A / double-click |
| Zoom around cursor or selection | wheel or pinch |
| Pan | horizontal scroll, ⇧ + wheel, or drag the ruler |
| Scroll when zoomed in | drag the bar above the ruler |
| Zoom to fit / to selection | ⌘0, ⌘E |
| Cut / copy / paste / delete | ⌘X ⌘C ⌘V, Delete |
| Trim, normalize | ⌘T, ⌘N |
| Undo / redo | ⌘Z, ⇧⌘Z |
| Open, save, save as | ⌘O, ⌘S, ⇧⌘S |

On Windows and Linux use Ctrl in place of ⌘.

## Layout

```
src/
  audio/      clip buffer, decode, encode, resample, normalize, fades
  analysis/   clip/zero/discontinuity scan, LUFS, band spectrum, ISO 226, meter ballistics
  engine/     audio worker thread: device enumeration, playback, recording, shared meters
  plugins/    plugin trait, scanner, processing stack, CLAP host, VST2 host, VST3 stub
  document/   clip + selection + undo history
  app/        egui UI: waveform, transport, meters, tonal balance, plugin panel, dialogs
```

## Analysis cache

Opening a file writes a sidecar `<file>.<ext>.sbpk` next to it (or under the OS cache
directory if that folder is read-only). It holds the waveform peak mipmap, the highlight
scan, loudness figures and the band spectrum, keyed to the file's size and modification
time, so reopening skips the analysis pass. Delete the sidecar at any time; it is rebuilt.

## Known limitations

- VST3 hosting is not implemented; the bundles appear in the browser so they can be
  wired up later without changing the UI.
- Plugin editors only open for CLAP plugins offering a *floating* window, which they create
  and own. Embedded editors — the common case, and every VST2 editor — need a native parent
  window Switchblade does not create yet, so those plugins still show generic sliders.
- Beat detection is aimed at percussive material. On sustained material the onset envelope has
  little to latch onto and the weaker hits it reports are the normalised noise floor.
- The plugin stack runs stereo. Mono files are duplicated into both channels; extra
  channels pass through untouched. No latency compensation when rendering.
- MP3 and OGG export are not included (both need C encoders); use WAV, AIFF or FLAC.
- DirectSound is not a cpal backend; on Windows, WASAPI is the default and ASIO is optional.
- Release builds are GUI-subsystem, so log output only appears when launched from a terminal
  (the app reattaches to the parent console) or from a debug build.
