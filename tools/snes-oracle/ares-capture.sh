#!/usr/bin/env bash
# ares headless capture: ares-capture.sh <rom> <frames> <outdir>
#
# Writes <outdir>/frame.raw   last captured RGBA8 frame (564x242 for NTSC)
#        <outdir>/audio.f32   whole run, interleaved stereo f32 at $ARES_AUDIO_RATE
#        <outdir>/trace.txt   per-frame FNV hashes (video, audio)
#
# Requires the local patched build (see README.md). Runs ares under cage with a
# headless Wayland backend, waits for the requested trace frame and a stable
# frame dump, then terminates the (never-exiting) GTK loop.
set -euo pipefail

rom=${1:?usage: ares-capture.sh <rom> <frames> <outdir>}
frames=${2:?}
out=${3:?}
mkdir -p "$out"

ares=${ARES:-/var/home/Lordra/ai/refs/ares/build_headless/desktop-ui/ares}
system=${ARES_SYSTEM:-Super Famicom}
rate=${ARES_AUDIO_RATE:-32000}

trace=$out/trace.txt
frame=$out/frame.raw
audio=$out/audio.f32
rm -f "$trace" "$frame" "$audio"
last=$((frames - 1))

env WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
    LIBGL_ALWAYS_SOFTWARE=1 ARES_AUDIO_RATE="$rate" \
    cage -- "$ares" --system "$system" --no-file-prompt --frames "$frames" \
      --dump-trace "$trace" --dump-frame "$frame" --dump-audio "$audio" \
      "$rom" >/dev/null 2>&1 &
pid=$!

prev=0
for _ in $(seq 1 20000); do
  if grep -q "^frame $last " "$trace" 2>/dev/null; then
    cur=$(stat -c %s "$frame" 2>/dev/null || echo 0)
    if [ "$cur" -gt 0 ] && [ "$cur" -eq "$prev" ]; then break; fi
    prev=$cur
  fi
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  sleep 0.05
done
sleep 0.2
kill -TERM "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true

test -s "$frame" || { echo "ares: frame dump missing ($frame)" >&2; exit 1; }
test -s "$trace" || { echo "ares: trace missing ($trace)" >&2; exit 1; }
ls -l "$frame" "$audio" "$trace"
