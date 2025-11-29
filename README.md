
# libffmpeg

## Feature flags
- `hwaccel` - Enabled by default
  - On linux, requires `libdrm-dev`
  - Enables support for automatically-detecting hardware acceleration, with detection for CUDA. (Disablable by setting `FFMPEG_DISABLE_HWACCEL`)
