# unko-snes-host

Host adapter between the standalone [`unko_snes`] core and [`unko_core`], the
system-agnostic emulator interface used by the Unko frontend and CLI.

`unko-snes` has no `unko-core` dependency; this crate owns every conversion
between the host-facing configuration types and the native SNES types.
