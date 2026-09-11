# Feature backlog

Ideas queued for Switchblade after v1. Unordered.

- **Beat detection**: reuse the beat detection code from m8-groove to detect hits and BPM.
- **Snap to beat**: cursor and selection edges snap to detected hits when enabled.
- **Fix discontinuities**: a function that repairs flagged sample jumps (short crossfade or
  interpolation across the jump) instead of only highlighting them.
- **DC removal**: measure and remove per-channel DC offset.
- **Hann-window crossfade loop**: seamless loop points by crossfading the loop boundary with a
  Hann window.
- **Loop metadata**: write loop points into the saved file (WAV `smpl` chunk, AIFF `INST`/`MARK`).
- **Highest possible internal precision**: process in f64 end to end and dither only on export.
