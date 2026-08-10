# Lumen fusion — knowledge base

Living document for **humans and agents** building 16→1 combine on top of `lri-rs`.  
Add findings in small PRs; link code paths and protos; mark confidence.

**Maintained by:** isamarin × BLMK  
**Status:** product on libcp (Luminat M0–M4 shipped) · open engine still grayscale MVP  
**Last updated:** 2026-08-10 (roadmap synced to code; Phase 3 MVP, Phase 4 export, Luminat M1–M4)

**L16 archive (submodule):** [`vendor/light-l16`](vendor/light-l16) → [github.com/isamarin/light-l16](https://github.com/isamarin/light-l16)

Clone with submodules: `git clone --recurse-submodules …` or `git submodule update --init`.

---

## Goal

Reconstruct the **single output image** the Light L16 / Lumen desktop app produced — not just 16 per-module DNGs.

Pipeline sketch:

```
.lri decode → per-module RAW (done)
           → shot metadata + calibration (in progress)
           → warp modules to common frame
           → depth map (ToF + disparity + focus)
           → exposure/colour match
           → blend (the hard part)
           → optional tone/HDR (view_preferences)
```

---

## What we already have (this fork)

| Layer | Status | Code |
| ----- | ------ | ---- |
| Block parse, zero-copy RAW | Done | `lri-rs/src/lib.rs`, `block.rs` |
| 10 bpp unpack, Bayer JPEG | Done | `unpack.rs`, `bayer_jpeg.rs` |
| Per-module DNG export | Done | `light/src/dng.rs`, `extract.rs` |
| Colour matrices (D65/F7) | Read | `block.rs` → `ColorInfo` |
| Sensor black/white | Read | `sensor_data` → `levels_for()` |
| Shot prefs (HDR, AWB, exposure) | Read | `ViewPreferences` |
| **Fusion metadata extract** | Done (v1) | `lri-rs/src/fusion.rs` → `LriFile.fusion` |
| GUI / gather shows fusion summary | Done | `light gather`, `api::LriSummary`, Lumen meta |

---

## Data in `.lri` relevant to fusion

Protobuf sources: [`lri-proto/proto/`](lri-proto/proto/)

### Per module (`LightHeader.module_calibration[]`)

| Proto field | Fusion role | Extracted? |
| ----------- | ----------- | ---------- |
| `geometry` (`GeometricCalibration`) | **Core** — intrinsics, extrinsics, mirror model | Yes (summary + K/R/t) |
| `geometry.per_focus_calibration[]` | Focus-dependent pose | Yes (list) |
| `geometry.distortion` | Lens model (polynomial / CRA) | Flags + coeffs when present |
| `geometry.extrinsics.moveable_mirror` | Mirror actuator + `MirrorSystem` | Flag + mirror pose |
| `vignetting` | Flat-field correction before blend | Flag only |
| `hot_pixel_map` / `dead_pixel_map` | Defect masking | Not yet |
| `color` | Per-module colour | Yes (`ColorInfo`) |

Key proto: [`geometric_calibration.proto`](lri-proto/proto/geometric_calibration.proto), [`mirror_system.proto`](lri-proto/proto/mirror_system.proto), [`distortion.proto`](lri-proto/proto/distortion.proto)

### Per shot (`LightHeader`)

| Field | Fusion role | Extracted? |
| ----- | ----------- | ---------- |
| `image_focal_length` | Pick `per_focus_calibration` bundle | Yes (`LriFile.focal_length`) |
| `image_reference_camera` | Reference module for alignment | Yes |
| `af_info` | Focus lock quality | Partial (`af_achieved`) |
| `tof_range` | Metric depth hint (metres?) | Yes |
| `device_calibration.tof` | Factory ToF linearization | Yes (offset/xtalk) |
| `imu_data[]` | Rolling shutter / motion | Sample counts |
| `gps_data` | Geo only (not geometry) | Lat/lon when present |
| `face_data` | Portrait ROI / weights? | Not yet |
| `view_preferences` | HDR tone, crop, orientation | Partial (no crop yet) |
| `proximity_sensors` | Near-field? | Not yet |
| `flash_data` | Flash modules | Not yet |

### Blocks not in LightHeader

| Block | Notes |
| ----- | ----- |
| `GPSData` (type 2) | Standalone GPS; parser skips protobuf parse on fast path — embedded `gps_data` in LightHeader is preferred |

---

## L16 hardware (why geometry is weird)

- **16 modules** in 3 rows (A1–A5, B1–B5, C1–C6); see [LRI.md](LRI.md) module grid.
- Many modules use a **movable mirror** (`MirrorType.MOVABLE`) — pose depends on focus/hall code, not fixed extrinsics.
- **ToF** sensor gives coarse depth; **multi-focus** modules give fine depth via disparity (hypothesis — verify against Lumen binary / papers).
- **Mono modules** (C row): likely used for depth / luminance, not colour.

Open questions — fill in when confirmed:

- [ ] Exact unit of `tof_range` (metres vs mm vs dioptres)
- [ ] How Lumen picks one `per_focus_calibration` entry for a given `image_focal_length`
- [ ] Whether `imu_data.frame_index` aligns to sensor row readout
- [ ] Role of `angle_optical_center_mapping` in final projection
- [ ] Output resolution and crop of the fused image vs single module

---

## Implementation roadmap

### Phase 0 — Inventory (current)

- [x] Document protos and gaps (this file)
- [x] Extract `FusionMeta` into `LriFile` (`geometry`, ToF, IMU, GPS)
- [x] Surface summary in `light gather` / API / Lumen

### Phase 1 — Calibration access

- [ ] Full `Distortion` + `VignettingCharacterization` structs (CRA radial done, polynomial + vignetting pending)
- [x] `MirrorSystem` numeric extract + movable-mirror R/t (`mirror_pose.rs`)
- [ ] `MirrorActuatorMapping` numeric extract
- [x] Select focus bundle: match `image_focal_length` to `per_focus_calibration` (`fusion::pick_focus_bundle`)
- [x] Export fusion JSON sidecar next to DNG export (`light extract` → `fusion.json`)

### Phase 2 — Geometric warp

- [x] Project module RAW → common reference plane (`image_reference_camera`)
- [x] Apply distortion (CRA radial) + mirror model — `distortion.rs`, `mirror_pose.rs`, `warp.rs`
- [ ] IMU-based row timing correction (if needed)

### Phase 3 — Depth + blend

- [x] ToF-guided coarse depth (`fuse::depth_range_from_tof` centres the sweep on the metric hint)
- [x] Multi-module plane-sweep depth (`stereo::plane_sweep`) — **MVP, grayscale**
- [ ] Dense per-pixel WarpField / SGM instead of a single sweep plane
- [x] Blend with per-pixel accumulation + masking (`fuse::accumulate_masked`, `normalize_blend`)
- [ ] Occlusion + confidence weights (`DepthAndOcc`, `confidence` — see libcp RE below)
- [ ] **Colour**: per-module debayer → linear RGB → colour matrices → vignetting flat-field →
      exposure/WB harmonization. *This is the main remaining gap to a self-owned engine.*

### Phase 4 — Output

- [x] Fused full-res TIFF / DNG export with view crop (`fuse_export.rs`, `tiff_out.rs`)
- [x] Match Lumen crop / orientation from `view_preferences`

### Phase 5 — Product (see [LUMEN_PLAN.md](LUMEN_PLAN.md))

Luminat M0–M4 landed 2026-08-05: Tauri GUI, libcp Render (profile 1/3), ADB camera pull,
batch, `.app` packaging, aperture / click-refocus / depth map. Quality 16→1 is **libcp-only**
by design; the in-tree fuse above stays the open, experimental track.

---

## External references

| Resource | Notes |
| -------- | ----- |
| **[isamarin/light-l16](https://github.com/isamarin/light-l16)** | **Git submodule** at [`vendor/light-l16/`](vendor/light-l16/) — maintained L16 archive (fork of helloavo) |
| [helloavo/Light-L16-Archive](https://github.com/helloavo/Light-L16-Archive) | Upstream archive; isamarin fork is the working copy in this repo |
| [isamarin/lri-rs](https://github.com/isamarin/lri-rs) | This decoder / GUI / fusion R&D repo |
| [dllu/lri-rs](https://github.com/dllu/lri-rs) / [gennyble/lri-rs](https://github.com/gennyble/lri-rs) | Proto extraction basis |
| [LRI.md](LRI.md) | Container format |
| [bayer_jpeg.md](bayer_jpeg.md) | BJPG decode |
| [openlight-camera](https://github.com/helloavo/openlight-camera) | Decompiled `light_camera.apk` (v1.3.5.1) — **writes** `.lri`, IPC to fusion service; local clone at `/Users/igor/StudioProjects/openlight-camera` |
| Light L16 Discord (linked from archive README) | Owner reports, firmware |
| Wayback [light.co/camera](https://web.archive.org/web/20191222062257/https://light.co/camera) | Marketing / spec claims |

### `vendor/light-l16/` — subfolders to mine (agents: check these)

| Path | Contents | Fusion relevance |
| ---- | -------- | ---------------- |
| [`Lumen/`](vendor/light-l16/Lumen/) | `Lumen-2.3.0.606.dmg` — **`libcp.dylib`** (CIAPI) | Combine pipeline RE — see log entry 2026-07-14 |
| [`Hardware/`](vendor/light-l16/Hardware/) | Exploded view, sensor layout | Physical module positions vs `GeometricCalibration` |
| [`Guides/`](vendor/light-l16/Guides/) | L16 photography blog clone | Capture behaviour, marketing claims |
| [`APKs/`](vendor/light-l16/APKs/) | Camera / Gallery apps | **Gallery** = fusion host (`libcp.so`, `LriProcessorService`); **camera** = `.lri` writer + AIDL client stubs only |
| [`L16 Lightroom Preset/`](vendor/light-l16/L16%20Lightroom%20Preset/) | Colour presets | Post-fusion look (not geometry) |

Firmware **1.3.5.1** OTA: `LFC-1351-0-00WW-A01-update.zip` ([helloavo release](https://github.com/helloavo/Light-L16-Archive/releases/tag/1.3.5.1)). Convert `system.new.dat` + `system.transfer.list` with `sdat2img.py` → ext4 (`e2ls` / `e2cp`).

---

## On-device architecture (firmware 1.3.5.1)

**Confidence:** confirmed (full `system` partition walk via `e2ls` on converted OTA image)

### `/system/priv-app/` — Light packages only

| Path | Package | Role |
| ---- | ------- | ---- |
| `light_camera/light_camera.apk` | `light.co.lightcamera` | Capture → `.lri` + LELR tail; **IPC client** to processing service |
| `light_display/light_display.apk` | `light.co.lightdisplay` | Display / UI shell |
| `light_gallery/light_gallery.apk` | `light.co.lightgallery` | **16→1 fusion** via `libcp.so` + `LriProcessorService` |

**Absent:** `lightprocessingservice.apk`, package `co.light.lightprocessingservice` (also missing from [`packages.txt`](vendor/light-l16/APKs/packages.txt)).

### `light_camera` — client only, no `IProcessor` server

| Component | Path | Notes |
| --------- | ---- | ----- |
| AIDL contract | `co/light/lightprocessingservice/IProcessor*.class` | Interface + `Stub` + `Proxy` only — **no** `Processor extends IProcessor.Stub` |
| IPC client | `light/co/lightsdk/process/Processor.class` | `bindService(Intent("co.light.lightprocessingservice.Processor"))` |
| On-device package | `co.light.lightprocessingservice` | openlight-camera RE renames to `co.openlight.*` |

### `light_gallery` — actual fusion host

```
LriProcessorService (internal, exported=false)
  └─ LriProcessor
       └─ ItemProcessor (per MediaItem)
            prepare → render(level) → waitForDepthMap → saveItem
            └─ LibCpRenderer (JNI → libnative-lib.so → CIAPI::Renderer)
```

**Native libs** (arm64): `libcp.so` (~7.25 MB), `libceres.so`, `liblricompression.so`, `libnative-lib.so`.

**JNI entry points** (`LibCpRenderer` / `libnative-lib.so`): `nativeObtainRenderer`, `nativePrepareRenderer`, `nativeRender`, `nativeSaveImage`, `nativeSetDofDepth`, `nativeGetDepthAtPoint`, `nativeReleaseRenderer`, …

**Process log strings** (from `light_gallery.odex`): `[PROCESS] start fusion and save`, `error from prepare`, `error from depth map render`, `saved jpeg`, `[TIMING] Time to process`.

### `libcp.so` (ARM) vs `libcp.dylib` (macOS Lumen 2.3.0.606)

| | ARM (`light_gallery.apk`) | macOS (`Lumen.app`) |
| - | ------------------------- | ------------------- |
| Size | 7 254 824 B | 6 935 696 B |
| Symbols | stripped ELF (nm needs `-D` on host) | exported (`nm -gU`) |
| Pipeline log strings | 105 | 104 — **same CIAPI messages** (SGM, ComputeFlowField, MonoFusion, GDepth, …) |
| STL ABI | `std::__ndk1` (Android NDK) | `std::__1` (libc++) |

**Implication:** desktop Lumen and on-device gallery share one engine; RE Lumen `libcp.dylib` is the primary offline reference. Do not chase `lightprocessingservice.apk` on 1.3.5.1.

### Work without a physical L16

1. RE `libcp.dylib` + gallery odex (pipeline order, CIAPI properties).
2. Run `light gather` / `light extract` on any `.lri` (Lumen export, archives, Discord).
3. Compare Lumen output (`*_1.jpg`) vs future `lri-rs` combine.
4. Camera + ADB only for runtime verification — **not** required for decode or gallery RE.

---

## Agent instructions

When working on fusion:

1. Read this file + [`LRI.md`](LRI.md) + relevant `.proto`.
2. Check **`vendor/light-l16/`** for hardware docs, Lumen binaries, guides — cite paths as `vendor/light-l16/...`.
3. Prefer **extract → unit test on real `.lri`** → document finding here.
4. Add a row to the tables above with **confidence**: `confirmed` / `likely` / `guess`.
5. Link PRs and file paths; keep prose short.
6. Do not block DNG/export work on fusion — land extractions incrementally.

### Entry template

```markdown
### YYYY-MM-DD — Short title

**Confidence:** likely  
**Source:** `path` or URL  
**Finding:** …  
**Implication for combine:** …  
**Follow-up:** …
```

---

## Log

### 2026-07-14 — Geometry lives in module_calibration

**Confidence:** confirmed  
**Source:** `lri-proto/proto/lightheader.proto`, `geometric_calibration.proto`  
**Finding:** Each `FactoryModuleCalibration` may carry full `GeometricCalibration` (intrinsics K, extrinsics R/t, per-focus bundles, movable mirror, distortion). Parser previously read only `color` from this message.  
**Implication:** Fusion does not need external calibration files — data is per capture in `.lri`.  
**Follow-up:** Implement focus-bundle selection; dump K/R/t in gather; compare across two `.lri` from same device.

### 2026-07-14 — No open-source combine implementation

**Confidence:** confirmed  
**Source:** This repo (`prism/`, `lri-study/`), helloavo archive README  
**Finding:** Community work stops at per-module extract; Lumen desktop did fusion closed-source.  
**Implication:** Green-field R&D; archive may help reverse-engineer, not copy-paste.  
**Follow-up:** Inspect archived Lumen app for algorithms / GPU shaders / log strings.

### 2026-07-14 — Fusion JSON sidecar on extract

**Confidence:** confirmed  
**Source:** `light/src/fusion_sidecar.rs`, `light extract`  
**Finding:** `light extract` now writes `fusion.json` next to per-module DNGs: shot ToF/IMU/GPS, reference camera, focal length, and full per-module geometry (K/R/t per focus bundle, mirror type, distortion flags).  
**Implication:** Any `.lri` (Lumen export, archives) can be validated for geometry completeness without a camera.  
**Follow-up:** [ ] Pick nearest `per_focus_calibration` for `image_focal_length` in sidecar; [ ] compare sidecar vs Lumen `.lumen` state fields.

### 2026-07-14 — FusionMeta extraction landed

**Confidence:** confirmed  
**Source:** `lri-rs/src/fusion.rs`, `block.rs`, `light gather`  
**Finding:** Per-module `GeometricCalibration` (K/R/t per focus bundle, mirror type, distortion flags), shot `tof_range`, factory ToF cal, IMU sample counts, GPS fix now populate `LriFile.fusion`.  
**Implication:** First real `.lri` can validate whether all 16 modules ship geometry.  
**Follow-up:** Dump nearest focus bundle for `image_focal_length`; JSON sidecar export.

### 2026-07-14 — light-l16 archive as git submodule

**Confidence:** confirmed  
**Source:** `vendor/light-l16/`, [github.com/isamarin/light-l16](https://github.com/isamarin/light-l16)  
**Finding:** Fork of helloavo/Light-L16-Archive vendored at `vendor/light-l16/` (APKs, Guides, Hardware, Lumen desktop, Lightroom presets). Primary offline reference for RE and fusion archaeology.  
**Implication:** Agents and humans should search the submodule before the web; pin findings to paths under `vendor/light-l16/`.  
**Follow-up:** Mirror firmware 1.3.5.1 release in isamarin fork; strings/symbols pass on `vendor/light-l16/Lumen/`.

### 2026-07-14 — Purchase checklist (offline pipeline)

**Confidence:** confirmed (for this fork)  
**Finding:** Seller must demonstrate: boots → shoots → `.lri` on disk without Light cloud. All 16 modules present in file.  
**Implication:** Brick risk is activation/cloud, not decode — once you have `.lri`, this repo handles RAW.

### 2026-07-14 — Lumen combine reverse-engineered from `libcp.dylib` (CIAPI)

**Confidence:** confirmed (symbols + log strings); pipeline **order** is `likely` (inferred from deps + error messages, not a recovered call graph)  
**Source:** [`vendor/light-l16/Lumen/Lumen-2.3.0.606.dmg`](vendor/light-l16/Lumen/Lumen-2.3.0.606.dmg) → `Lumen.app/Contents/Frameworks/libcp.dylib` (~6.9 MB), `libceres.dylib`  
**Tools:** `nm -gU | c++filt`, `strings`, `otool -L` on macOS (verified 2026-07-14 in this repo)

**Verified strings (sample):** `Cannot process undistortion without Stereo!`, `ComputeFlowField only configured for 3-6 pyramid levels!`, `Number of flow fields should match number of source images!`, `SGM after upsampled depth is not allowed.`, `Super-res does not support mono modules!`, `Effective focal length must be larger than reference focal length!`, `GDepth:Format="RangeInverse"`, `DepthAndOcc`, `ReferenceImageCache not implemented for mono camera!`  
**Verified symbols:** `CIAPI::RendererBase::setInputDataStream`, `CIAPI::DirectRenderer`, `CIAPI::ImagePyramid`, `CIAPI::DepthEditor::*`, `lt::ComputeFlowField`, `lt::StereoLayer`, `lt::ReferenceImageCache`, `lt::MonoFusion`, `ceres::AutoDiffCostFunction<lt::Internal::ReProjectionCost…>`, Halide runtime (`halide_runtime`, `Halide::Runtime::Internal::*`)

**Stack:** Qt5/QML front-end · **Ceres** (non-linear least squares) · **Halide** (compiled compute kernels; `_halide_*`, 3–6 pyramid levels) · libjpeg-turbo (`jsimd`) · proprietary **`libcp.dylib`** exposing namespace **`CIAPI`** (Computational Imaging API) · `liblricompression` (codec — already reimplemented here). Internal build subsystems (from leaked Jenkins paths): `camera` (dominant), `stereo`, `3rdparty`.

**API spine (`CIAPI::Renderer` / `RendererBase` / `DirectRenderer`):**
`setInputDataStream(.lri bytes)` → `render(level, ROI, RenderType)` → `outputBuffer()` / `writeImage(stream, size, ExportImageFormat)`. Property bag (`ParamFloat/Int/String/…`), `serialize/deserialize(StateType)` = the `.lumen` state file, `setOutputUpdateListener(ImagePyramid, ROI, level)` (progressive tiled output). Desktop-only depth features (`Renderer in Desktop profile`).

**Reconstructed pipeline (`likely` order, each stage name = confirmed string):**

```
.lri → per-module RAW + GeometricCalibration        (we have this)
     → Stereo: undistort → SGM disparity, coarse→fine over 3–6 level ImagePyramid,
       seeded by ToF → depth                         ("Cannot process undistortion without Stereo",
                                                       "SGM after upsampled depth is not allowed")
     → ComputeFlowField: one dense flow field per source module
                                                      ("Number of flow fields should match number of source images",
                                                       "ComputeFlowField only configured for 3-6 pyramid levels")
     → warp/resample each module into reference-camera frame using depth+flow (Halide)
                                                      (reference = widest module: "Effective focal length
                                                       must be larger than reference focal length")
     → Super-res: color/tele modules add detail onto wide reference (mono excluded)
                                                      ("Super-res does not support mono modules")
     → Blend with occlusion + confidence weights      ("DepthAndOcc", "confidence")
     → Ceres refine (poses/alignment; residuals ↔ proto reprojection_error/stereo_error)
     → DepthEditor (brush/heal/lasso/quick-select/face matte) → re-render → Refocus (synthetic bokeh)
     → writeImage; depth exportable as Google GDepth XMP, Format="RangeInverse" (inverse-range)
```

**Confirmed facts for the rebuild:**
- Depth is the spine and is **computed per-shot by SGM stereo**, not just read from ToF — ToF is a seed/prior. Calibration in `.lri` is the initialization; Ceres re-solves (matches `rms_error`/`stereo_error`/`reprojection_error` proto fields).
- **Reference module = widest (28 mm-equiv)**; tele (70/150) modules are warped in and super-resolve detail onto it.
- **Mono (C-row) modules**: feed depth + luminance; **excluded from super-res** ("Empty mono!", "ReferenceImageCache not implemented for mono camera", panchromatic noise cal).
- Depth stored **inverse-range** (disparity-like); Lumen embeds Google Photos depthmap XMP.
- Multi-scale everywhere: **3–6 level image pyramids** for both SGM and flow.

**Implication for combine:** the "hard part" is now named — implement, in order: (1) undistort via extracted `Distortion`; (2) **pyramidal SGM disparity** between overlapping modules using K/R/t (+ ToF seed) → inverse-range depth; (3) per-module **dense flow** refine; (4) **depth-guided warp** all modules to the widest reference; (5) **occlusion-aware multiband blend**, color modules super-res, mono → luma. Ceres is optional for an MVP (use calibration poses directly).

**Follow-up:**
- [ ] `nm`/`strings` pass on the `camera` subsystem symbols (module → sensor-type → focal-group mapping).
- [ ] Confirm SGM param / pyramid-level count vs focal group.
- [ ] Validate on a real `.lri`: do all 16 modules carry per-focus geometry? Is depth per-focus?
- [ ] Prototype: undistort + 2-view SGM (reference 28 mm ↔ one 70 mm) → depth → warp → feather blend, as the smallest end-to-end slice.
- [ ] Extract `stereo_state.proto` from binary strings / archive if not already in protos.

### 2026-07-14 — `light_camera.apk` writes `.lri` but does not fuse

**Confidence:** confirmed  
**Source:** [openlight-camera](https://github.com/helloavo/openlight-camera) (`light_camera.apk` v1.3.5.1_0118), smali `openlight/co/camera/utils/ImageUtil$ImageSaver`, `MediaFileManager`  
**Finding:** Camera app captures vendor RAW via `ImageReader` (`CameraInfo.getRawFormat()`), streams bytes to `L16_NNNNN.lri` under `/DCIM/Camera/`. After the HAL blob it **appends** two `LELR` blocks: `message_type=1` (`ltpb.ViewPreferences`) then `message_type=2` (`ltpb.GPSData` with live location + EXIF-aligned timestamps). JPEG sidecar (`0x100`) gets EXIF only — no tail blocks. Processed gallery JPEG is `*_1.jpg`.  
**Implication:** `lri-rs` block parser matches on-device writer byte-for-byte; geometry / 16-module payload comes from the HAL/native stack in the leading blob, not from Java.  
**Follow-up:** Compare appended `ViewPreferences` fields vs nested `LightHeader.view_preferences` on real captures.

### 2026-07-14 — On-device fusion IPC (`ProcessRequest` + `Processor` client)

**Confidence:** confirmed (AIDL + client); server APK **absent** on 1.3.5.1  
**Source:** `co/light/lightprocessingservice/*` (stubs in `light_camera.odex`), `light/co/lightsdk/process/Processor` (client in camera odex); openlight-camera RE uses renamed package `co.openlight.*`  
**Finding:** `light_camera` ships **AIDL stubs only** (`IProcessor`, `IProcessor$Stub`, `ProcessRequest`, …) — **no** class `Processor extends IProcessor.Stub`. Client `lightsdk.process.Processor` binds `Intent("co.light.lightprocessingservice.Processor").setPackage("co.light.lightprocessingservice")` → `IProcessor.createProcessedImage(ProcessRequest)`. Request fields: `mLriPath`, `ProcessingProfile` (`THUMBNAIL=0` … `DESKTOP=3`), `ProcessingLevel` (`ZERO`/`ONE`/`TWO`), `mBokeh` (f/2–f/15 or 0), `focusDepthPoint(x,y)` ∈ [-1,1], `enabledSuperRes`, `gDepth` (**DESKTOP only**), JPEG/DNG output paths, `postProcessingComplete()`.  
**Implication:** `ProcessRequest` is the parcelable façade over CIAPI knobs; the **server was never shipped** as a separate APK on 1.3.5.1 — gallery's `LriProcessorService` + `libcp.so` is the on-device implementation path.  
**Follow-up:** [ ] Trace who calls `lightsdk.process.Processor.processImage()` in camera odex (if at all on retail builds); [ ] map `ProcessRequest` fields 1:1 to gallery `LibCpRenderer` setters (partially done — see profile mapping entry).

### 2026-07-14 — Gallery hosts fusion: `libcp.so` + `LriProcessorService`

**Confidence:** confirmed  
**Source:** `vendor/light-l16/APKs/Original APKs/light_gallery/light_gallery.apk`, decompiled classes under `vendor/light-l16/APKs/light_gallery_decompiled/light/co/gallery/`  
**Finding:** Gallery APK (v1.3.5.1, `android.uid.system`) bundles **`libcp.so` (~6.9 MB)**, `libceres.so`, `liblricompression.so`, `libnative-lib.so` — same CIAPI stack as Lumen `libcp.dylib`. Manifest declares internal service `light.co.gallery.LriProcessorService` (not exported). Java pipeline: `LriProcessor` → `ItemProcessor` per `MediaItem`: `prepare` → `render` → `waitForDepthMap` → `saveItem` (JPEG + optional `.dng` + state) → `LriCompressor.compressLriFile`. JNI class `light.co.gallery.utils.LibCpRenderer` wraps `CIAPI::Renderer` (`nativeObtainRenderer`, `nativePrepareRenderer`, `nativeRender`, `nativeSaveImage`, `nativeSetDofDepth`, `nativeGetDepthAtPoint`, …).  
**Implication:** On-device 16→1 is **not** implemented in Java — it's the same closed `libcp` engine as desktop Lumen, driven through `LibCpRenderer`. Gallery strings confirm UX: "calculating depth info", "processing bokeh", GDepth XMP (`Format="RangeInverse"` in `libcp.so`).  
**Follow-up:** [x] Diff ARM `libcp.so` vs macOS `libcp.dylib` — same pipeline log strings, ~320 KB larger on ARM, NDK vs libc++ mangling only; [ ] full baksmali of gallery odex (`.class` extracts in archive are corrupt for `javap`).

### 2026-07-14 — Profile / level mapping (JNI `map_profile`)

**Confidence:** confirmed (disassembly + enum constant pools)  
**Source:** `libnative-lib.so` (`_Z11map_profilei`), `LibCpRenderer$Profile`, `database.models.ProcessLevel`  
**Finding:** `map_profile(int)` clamps to max **3** then passes to `CIAPI::Renderer::Create(RendererProfile)` — **1:1** with `ProcessRequest.ProcessingProfile` ordinals (`THUMBNAIL=0` … `DESKTOP=3`). `LibCpRenderer$Profile` adds device-specific presets (`DEVICE_L16`, `FL5`, `DESKTOP_0`) with per-mode render levels (`getDesktopLevel`, `getBackgroundLevel`, `getPreviewLevel`). DB `ProcessLevel` enum: `THUMBNAIL`, `DEVICE_L16`, `DEVICE_FL5`, `DESKTOP_0`, `DESKTOP_1`. JNI maps `ProcessRequest` knobs to CIAPI params: `ViewDofFNumber` ← bokeh, `ViewDofFocusDepth` / `RefocusPoint` ← focus tap, `JPEG_GDEPTH` export when `gDepth` + DESKTOP. `ProcessingLevel` ZERO/ONE/TWO → `render(level=…)` pyramid tier (log: `render(level=%d, …)`).  
**Implication:** Reimplementing combine can mirror Lumen tiers: pick `RendererProfile` + pyramid `level` exactly as mobile/desktop profiles do; depth-map types (`DepthMapType`: `NONE`, `MONO`, `COLOR_1`, `COLOR_2`, `REFOCUS_MASK`) gate refocus/bokeh passes.  
**Follow-up:** [ ] Correlate `DESKTOP_0` vs `DESKTOP_1` with output resolution; [ ] `DepthMode` (`NONE`, `TOGGLE_APERTURE`, `USE_CURRENT`) vs `view_preferences`.

### 2026-07-14 — Firmware 1.3.5.1 system partition: no `lightprocessingservice`

**Confidence:** confirmed  
**Source:** OTA `LFC-1351-0-00WW-A01-update.zip` → `sdat2img.py` → `/tmp/system.img`; `e2ls /priv-app`; binary search (`lightprocessingservice.apk` offset −1); `packages.txt`  
**Finding:** `system/priv-app` contains only `light_camera`, `light_display`, `light_gallery`. String `lightprocessingservice.apk` absent from entire system image. All 20 `lightprocessingservice` string hits sit in `light_camera.odex` (AIDL class names + intent actions). Gallery odex has **zero** `co.light.lightprocessingservice` references.  
**Implication:** Chasing a standalone processing APK on 1.3.5.1 is a dead end; fusion R&D should target `light_gallery.apk` + Lumen `libcp.dylib`.  
**Follow-up:** [ ] Mirror extracted firmware APKs into `vendor/light-l16/APKs/Firmware-1.3.5.1/` for reproducibility.

### 2026-07-14 — `LightHeader` Java writer = `LELR` spec

**Confidence:** confirmed  
**Source:** `openlight/co/camera/proto/LightHeader.smali`  
**Finding:** Writer emits: magic `LELR` (4) · `block_length` u64 LE · `message_offset` u64 LE (= `HEADER_LENGTH`) · `message_length` u32 LE · `type` u8 · 7 zero reserved · protobuf bytes. `TYPE_VIEW_PREFS=1`, `TYPE_GPS_DATA=2`. `HEADER_LENGTH = 4+8+8+4+1+7 = 32`.  
**Implication:** Matches [`LRI.md`](LRI.md) and `lri-rs/src/block.rs::Header::ingest` — no Android-specific quirks.  
**Follow-up:** None.

### 2026-07-14 — Movable-mirror pose: `flip_img_around_x` handling was wrong

**Confidence:** confirmed  
**Source:** `lri-rs/src/mirror_pose.rs`, commit `cff7a7d`, NCC validation in `light/src/validate_rt.rs`  
**Finding:** Base R/t from `module_calibration` was already correct (B4↔B2/B3 pairs score NCC ≈ 0.4), but reflected poses for part of the movable set (B1, B5, all C) came out with NCC ≈ 0 or negative. Root cause was the `flip_img_around_x` branch in the mirror reflection, not the extrinsics convention.  
**Implication for combine:** Removes the last known geometry blocker before dense stereo — all movable modules now warp into the reference frame consistently.  
**Follow-up:** [ ] Re-measure NCC across the full 101-capture corpus once the golden set exists.

### 2026-07-14 — Phase 3 MVP: undistort → plane-sweep → warp → blend

**Confidence:** confirmed  
**Source:** `light/src/fuse.rs`, `lri-rs/src/{stereo,warp,distortion}.rs`, commits `7125539`, `df5d059`  
**Finding:** End-to-end in-tree fusion runs: CRA radial undistort (`LensUndistortCRA`), plane-sweep over a ToF-centred depth range, homography warp to the reference module, masked accumulation and normalized blend. Output is **grayscale** — the whole path operates on `GrayImage`.  
**Implication for combine:** Geometry and depth are now testable end-to-end, but this is not yet a Lumen substitute: colour (debayer → linear RGB → colour matrices → vignetting → exposure/WB harmonization) is entirely absent.  
**Follow-up:** [ ] Colour path. [ ] Replace single-plane sweep with dense per-pixel WarpField.

### 2026-07-14 — Phase 4: full-res TIFF/DNG export with view crop

**Confidence:** confirmed  
**Source:** `light/src/fuse_export.rs`, `light/src/tiff_out.rs`, commit `7a4f77b`  
**Finding:** Fused output writes full-resolution TIFF/DNG and honours crop/orientation from `view_preferences`.  
**Implication for combine:** Output stage is no longer a gap; quality is bounded by the fusion stage, not the writer.  
**Follow-up:** None.

### 2026-08-05 — Luminat M1–M4: libcp-first product shell

**Confidence:** confirmed  
**Source:** [`LUMEN_PLAN.md`](LUMEN_PLAN.md), `light/src/{libcp,camera,mono,config}.rs`, `lumen/`, `tools/libcp-export/`  
**Finding:** Product track reached M4. Tauri 2 GUI with Light-engine Render (profile 1 ≈ 13 MP / profile 3 DESKTOP 10432×7824), ADB camera list/pull, batch render, `.app` packaging with embedded `libcp-export`, and M4 extras reverse-engineered from CIAPI: `setProperty(ParamFloat)` FocusDepth=1 / FNumber=3, `DepthEditor` 16 B shared_ptr with `getDepthAtPoint(Point<float>)`. Smoke on `L16_00026`: f/4 + click (0.5, 0.4) at profile 3 → z ≈ 16829 mm, depth map 320×240 valid.  
**Implication for combine:** Quality 16→1 is intentionally **libcp-only** in the product; the in-tree fuse stays the open research track. This means Luminat currently depends on the user's own `Lumen.app` for `libcp.dylib` + `libceres.dylib` (x86_64, Rosetta) — a dependency the open engine is meant to eventually retire.  
**Follow-up:** [ ] Golden set: run all 101 captures through `libcp` (profiles 1 and 3, plus depth maps) and store outputs as ground truth. This both measures the open engine (SSIM/PSNR) and preserves libcp behaviour against the day Rosetta or Lumen.app stops working.
