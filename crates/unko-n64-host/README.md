# unko-n64-host

Adapter between the `unko-n64` scaffold and the shared `unko-core` interface.

Cartridge parsing and metadata are implemented; `run_frame` currently returns
`CoreError::Unsupported` because CPU/RCP emulation is the next milestone. The
core is registered so the host can select N64 images and show a precise status
instead of treating them as another system.
