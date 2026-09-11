# Feature backlog

Ideas queued for Switchblade after v1. Unordered.

- **Plugin editors, embedded**: CLAP plugins offering a floating window already open one, and
  the host side — `clap.gui`, `clap.thread-check`, real `host_data` — is in place. What is
  missing is a native parent window per editor (baseview, or hand-rolled Win32 / Cocoa / X11),
  which is what every embedded CLAP editor and every VST2 editor needs. Two snags remain: the
  editor has to take the stack mutex that the audio thread also takes, so opening one while
  playing briefly bypasses the stack; and `effEditIdle` is unreachable through the `vst` crate
  (its `dispatch` is private), so VST2 repaints need a fork or a direct dispatcher.
- **Beat-aware editing**: now that hits are detected, use them — trim to the nearest hit, split
  at every hit, or quantise a selection's start to the grid.
- **Fix the remaining f32 boundaries**: accumulators in the spectrum and meters are f64, but
  the clip, the resampler and the plugin stack are all f32 end to end. Moving the clip to f64
  would need the plugin bridge to convert per block, so it is only worth it if something
  measurable comes out of it.
- **Beat detection on selections**: detection runs over the whole file; running it on the
  selection would make it usable on a single bar of a long recording.
- **Tune beat sensitivity from the UI**: `BeatSettings::sensitivity` is ported and honoured,
  but nothing exposes it yet — a slider in the side panel would make sustained material usable.
