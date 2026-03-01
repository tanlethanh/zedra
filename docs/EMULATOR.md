# Android Emulator Support

Running Zedra on the Android emulator with wgpu/Vulkan rendering.

## Quick Start

```bash
# Launch emulator with host GPU (MoltenVK on Apple Silicon)
emulator -avd Pixel_3a_API_34 -gpu host -no-snapshot-load

# Build, install, and launch
cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib
cd android && ./gradlew installDebug -x buildRustLib && cd ..
adb shell am start -n dev.zedra.app/.MainActivity
```

The `-gpu host` flag is required. Other GPU modes do not work (see below).

## GPU Backend Investigation

### SwiftShader (default, `-gpu swiftshader_indirect`)

SwiftShader is the emulator's default software Vulkan renderer. It **hangs
indefinitely** on `vkCreateGraphicsPipelines` for our shaders. The SPIR-V →
LLVM machine code JIT compilation never completes for the complexity of the
GPUI shader module (~1300 lines WGSL, 9 render pipelines).

Tested: 8+ minutes on the first pipeline (`quads`) with no progress. The app
process stays alive but the main thread is blocked in the Vulkan driver.

**Verdict**: Not viable for Zedra.

### GLES / GL Backend (`-gpu angle_indirect`)

wgpu supports GLES 3.0+ as a backend, but the emulator's GLES implementation
(ANGLE) only exposes GLES 3.0, which lacks:

- **Storage Buffer Objects (SSBOs)**: Required for GPUI's instanced rendering.
  All primitive data (quads, sprites, etc.) is passed via `var<storage, read>`
  bindings. GLES 3.1+ is needed for SSBOs.
- **Compute shaders**: GLES 3.0 has no compute support.
- **Adequate texture size**: WebGL2 defaults cap `max_texture_dimension_2d` at
  2048, which is smaller than the emulator's surface (2220px tall).

**Verdict**: Dead end — GPUI fundamentally requires storage buffers.

### Lavapipe (`-gpu lavapipe`)

Lavapipe is a CPU-based Vulkan implementation (Mesa's software renderer). It
is **not available** in emulator version 36.2.12.0 — the flag is silently
ignored and the emulator falls back to `swangle_indirect`.

**Verdict**: Not available in current emulator releases.

### Host GPU (`-gpu host`) — Working

The `-gpu host` flag passes Vulkan calls through to the host machine's GPU
driver. On Apple Silicon Macs, this uses **MoltenVK** (Vulkan-to-Metal
translation layer) with the host's Metal GPU.

All 9 render pipelines compile in ~700ms (vs infinite hang on SwiftShader).
Frame draw times stabilize at 8-9ms after the first frame.

**Verdict**: Working. This is the only viable path for emulator testing.

## MoltenVK Compatibility Fixes

Three issues were discovered and fixed when running through MoltenVK:

### 1. SPIRV-Cross Array Type Mismatch

**File**: `vendor/zed/crates/gpui_wgpu/src/shaders.wgsl`

When naga compiles WGSL to SPIR-V and SPIRV-Cross translates that to Metal
Shading Language (MSL), fixed-size arrays in storage buffer structs don't match
function parameter types:

```
error: no matching function for call to 'prepare_gradient_color'
  no known conversion from 'const device LinearColorStop[2]'
  to 'spvUnsafeArray<LinearColorStop, 2>' for 4th argument
```

SPIRV-Cross wraps function parameters in `spvUnsafeArray<T, N>` but storage
buffer member access produces raw `const device T[N]`. There is no implicit
conversion between these types in MSL.

**Fix**: Changed `prepare_gradient_color` to accept individual `LinearColorStop`
elements instead of `array<LinearColorStop, 2>`:

```wgsl
// Before (fails on MoltenVK)
fn prepare_gradient_color(... colors: array<LinearColorStop, 2>) -> GradientColor

// After (works everywhere)
fn prepare_gradient_color(... stop0: LinearColorStop, stop1: LinearColorStop) -> GradientColor
```

### 2. Metal Built-in Name Conflict

**File**: `vendor/zed/crates/gpui_wgpu/src/shaders.wgsl`

The shader defines a custom `fmod` function (truncated modulus). When
SPIRV-Cross translates to MSL, this conflicts with Metal's built-in
`metal::fmod`, causing an ambiguous call error:

```
error: call to 'fmod' is ambiguous
  candidate: METAL_FUNC float fmod(float x, float y)  // built-in
  candidate: float fmod(float a, float b)              // ours
```

**Fix**: Renamed `fmod` → `trunc_mod`.

### 3. WGSL Directive Ordering

**File**: `vendor/zed/crates/gpui_wgpu/src/shaders_subpixel.wgsl`

WGSL requires `enable` directives before all global declarations. The subpixel
shader file started with `enable dual_source_blending;`, but when concatenated
after the base shader module (`base + subpixel`), the directive appeared after
hundreds of global declarations.

**Fix**: Removed the directive from the `.wgsl` file. The renderer now prepends
`enable dual_source_blending;\n` before the base shader when concatenating.

### 4. Dual-Source Blending on Emulator

**File**: `vendor/zed/crates/gpui_wgpu/src/wgpu_context.rs`

MoltenVK through the emulator's Vulkan proxy reports `DUAL_SOURCE_BLENDING` as
available, but requesting it in `DeviceDescriptor.required_features` triggers
an immediate device lost error.

**Fix**: Detect the Android emulator (via `/dev/goldfish_address_space`) and
skip requesting the DSB feature. Text rendering falls back to grayscale
anti-aliasing (same behavior as on Mali GPUs that lack DSB natively).

## Pre-compiled Shaders

Pre-compiling WGSL to SPIR-V at build time was investigated but would **not**
help with SwiftShader. The bottleneck is SPIR-V → LLVM machine code JIT, not
WGSL → SPIR-V translation (which takes only ~30ms). Pipeline caching
(`wgpu::PipelineCache`) would help subsequent launches on physical devices but
cannot help if the first compilation never completes.

## Performance on Emulator

With `-gpu host` on Apple M1 Pro:

| Metric | Value |
|--------|-------|
| Pipeline creation (all 8) | ~700ms |
| Shader module compilation | ~30ms |
| Frame draw (first) | ~33ms |
| Frame draw (steady state) | ~8-9ms |
| Memory (RSS) | ~246MB |
| GPU | Apple M1 Pro via MoltenVK |
| DSB | Disabled (emulator workaround) |

## Limitations

- **Physical device recommended**: The emulator is useful for UI iteration but
  performance characteristics differ significantly from real mobile GPUs.
  Always validate on a physical device for performance testing.
- **No snapshot support**: Use `-no-snapshot-load` to avoid stale GPU state.
- **Intel Macs**: Not tested. MoltenVK requires Metal support (2012+ Macs).
- **Linux hosts**: Would need a native Vulkan GPU. MoltenVK is macOS-only.

---

*Last updated: 2026-02-25*
