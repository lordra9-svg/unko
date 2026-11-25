# unko-snes-api

SNES-specific extension surface for the generic [`unko_core::Core`] interface.

`unko_core` owns the generic lifecycle (load, run, video, audio, input, save
states). This crate owns the SNES vocabulary layered on top: the controller
bit layout, the timing region, video-mode metadata, and the [`SnesCore`]
extension trait used by the host adapter.
