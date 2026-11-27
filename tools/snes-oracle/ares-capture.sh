#!/usr/bin/env bash
# ares headless capture: ares-capture.sh <rom> <frames> <outdir>
#
# Writes <outdir>/frame.raw        last captured RGBA8 frame (256x240)
#        <outdir>/frame.raw.idx    last frame as u16 NES colour indices
#        <outdir>/frame.raw.idxseq all frames as u16 NES colour indices
#        <outdir>/audio.f32        whole run, interleaved stereo f32 at $ARES_AUDIO_RATE
#
# Set ARES_RAW_AUDIO=<path> to also capture the S-DSP's pre-resampler i16
# output (see README.md); the local ares build must include that hook.
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

# Reproducible power-on RAM. ares seeds its WRAM garbage pattern from the host
# clock, so two runs of a game that reads uninitialised work RAM (Super Mario
# RPG does) differ by several percent of pixels. ARES_DETERMINISTIC selects
# ares' own "Deterministic Entropy" seed, which our core mirrors exactly
# (crates/unko-snes/src/random.rs). Captures must be comparable to mean
# anything, so this is always on.
export ARES_DETERMINISTIC=${ARES_DETERMINISTIC:-1}
rate=${ARES_AUDIO_RATE:-32040}

trace=$out/trace.txt
frame=$out/frame.raw
audio=$out/audio.f32
rm -f "$trace" "$frame" "$audio"
last=$((frames - 1))

# `setsid` puts cage and ares in their own process group so the whole tree can
# be signalled. The patched build exits by itself once the capture is written
# (`captureFinish(); _Exit(0)`); this teardown is only for a stalled run.
env WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
    LIBGL_ALWAYS_SOFTWARE=1 ARES_AUDIO_RATE="$rate" \
    setsid cage -- "$ares" --system "$system" --no-file-prompt --frames "$frames" \
      --dump-trace "$trace" --dump-frame "$frame" --dump-audio "$audio" \
      "$rom" >/dev/null 2>&1 &
pid=$!

# Give up (and clean up) rather than waiting forever on a stalled capture.
deadline=$((SECONDS + ${ARES_TIMEOUT:-600}))
prev=0
stalled=0
while :; do
  if grep -q "^frame $last " "$trace" 2>/dev/null; then
    cur=$(stat -c %s "$frame" 2>/dev/null || echo 0)
    if [ "$cur" -gt 0 ] && [ "$cur" -eq "$prev" ]; then break; fi
    prev=$cur
  fi
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  if [ "$SECONDS" -ge "$deadline" ]; then stalled=1; break; fi
  sleep 0.05
done
[ "$stalled" = 0 ] && sleep 0.2

# Terminate the whole group, escalating to KILL if it does not go away.
kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
for _ in $(seq 1 50); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.1
done
kill -KILL -"$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
if [ "$stalled" = 1 ]; then
  echo "ares: capture stalled after ${ARES_TIMEOUT:-600}s, killed" >&2
  exit 1
fi

test -s "$frame" || { echo "ares: frame dump missing ($frame)" >&2; exit 1; }
test -s "$trace" || { echo "ares: trace missing ($trace)" >&2; exit 1; }
ls -l "$frame" "$audio" "$trace"
