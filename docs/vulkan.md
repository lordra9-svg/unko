# Vulkan / wgpu renderer

Status: **started** (offscreen scaffold, not yet the default presentation path).

## What exists

`unko-app` has a `vulkan` feature:

```sh
cargo check  -p unko-app --features vulkan
cargo test   -p unko-app --features vulkan vulkan
```

`crates/unko-app/src/vulkan.rs` creates a Vulkan-backed `wgpu` device and an
offscreen pipeline that:

- uploads a 256x240 (or arbitrary) RGBA8 emulator frame into a GPU texture;
- draws it with a fullscreen triangle and a nearest-neighbour sampler;
- renders to an offscreen `Rgba8Unorm` target;
- copies the target into a mapped readback buffer and returns unpadded RGBA.

The test verifies a full frame round-trips byte-for-byte. If no Vulkan adapter
is available the test skips; `VulkanRenderer::new` returns an error so callers
can fall back to the existing glow path.

## Why it is not the default yet

The desktop shell still uses eframe's `glow` backend and the existing
`gpu_shader` GLSL presets. Switching presentation to `egui-wgpu`/wgpu is a
separate integration step: surface configuration, resize/scale handling,
texture upload from the core, and porting the NTSC/CRT shader presets to WGSL.
The offscreen slice proves the device, texture, pipeline and readback path
without destabilising the current renderer.

## Next steps

1. Select the renderer at startup (`glow` default, `wgpu`/Vulkan opt-in) and
   create a surface for the eframe window.
2. Upload the completed core frame directly to the GPU texture instead of the
   current CPU texture upload.
3. Port the Raw/Composite/CRT/NTSC presets to WGSL uniforms and compare output
   against the glow traces.
4. Add a headless benchmark and a fallback when no Vulkan adapter exists.

The existing performance budget and accuracy gates remain on the glow path
until the wgpu presenter is ready.
