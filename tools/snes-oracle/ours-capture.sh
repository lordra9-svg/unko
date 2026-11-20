#!/usr/bin/env bash
# Our headless capture: ours-capture.sh <rom> <frames> <outdir>
#
# Writes <outdir>/frame.raw  RGBA8 256x224
#        <outdir>/audio.f32  interleaved stereo f32 at the native 32 kHz
#
# Set IPL to a 64-byte SPC IPL ROM (required for the real boot protocol).
# Coprocessor titles also need their firmware supplied through the environment:
# CX4_ROM=<cx4.rom> for Mega Man X2/X3 and DSP1_ROM=<dsp1.rom> for the NEC DSP
# games. Without CX4_ROM a CX4 title's self-test fails and it never uploads its
# sound driver (silent output).
set -euo pipefail

rom=${1:?usage: ours-capture.sh <rom> <frames> <outdir>}
frames=${2:?}
out=${3:?}
mkdir -p "$out"

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ipl=${IPL:-/var/home/Lordra/ai/refs/ares/ares/System/Super\ Famicom/ipl.rom}

cd "$root"
FRAMES="$frames" FRAME_OUT="$out/frame.raw" AUDIO_OUT="$out/audio.f32" \
  cargo run -q --release -p unko-snes --example snes-info -- "$rom" "$ipl" \
  >"$out/info.txt" 2>&1

test -s "$out/frame.raw" || { echo "ours: frame dump missing" >&2; exit 1; }
ls -l "$out/frame.raw" "$out/audio.f32" 2>/dev/null || true
