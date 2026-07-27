# hal-capture — the on-camera app and the `co.light` HAL contract

Everything else in this repo is read off the **output** side: `.lri` files parsed
after the fact. This document is the **input** side — what the camera app asks the
hardware to do, read off a live L16 over USB. It is a different evidence base than
the container parsing, which is exactly why the places where the two agree are
worth trusting more than either alone.

**Source & method.** Firmware 1.3.5.1 reference unit, Android 6.0.1 (API 23), over
ADB. Vendor tags from `dumpsys media.camera`; app logic from `jadx`/`apktool` on
`light.co.lightcamera` (v1.3.5.1_0118_00025263, versionCode 10); native symbols
from `objdump -T | c++filt`. The recompiled/annotated app lives in the sibling
`openlight-camera` project.

Confidence tags as elsewhere: **[obs]** directly observed on the device,
**[inf]** inferred, **[?]** open.

---

## 1. The `co.light` vendor-tag contract  [obs]

The camera service exposes a private vendor-tag section `co.light` (18 tags,
base `0x80000000`). The Java app reaches them by reflecting the hidden
`CaptureRequest.Key(String, Class)` constructor and caching by identity — the
public camera2 API never sees them. Full descriptor, verbatim from
`dumpsys media.camera`:

### Control (request) side

| tag | type | meaning |
| --- | --- | --- |
| `zoom_factor` | float | selects focal / module combination |
| `iso_range_min` / `_max` | int32 | ISO priority |
| `shutter_range_min` / `_max` | int64 | shutter priority (ns) |
| `focus_type` | int32 | focus mode |
| `stacked_capture_state` | byte | multi-module frame stacking |
| `stacked_capture_fw` | byte | firmware-side stacking |
| `tripod_fw` | byte | tripod / long-exposure mode |
| `burst_fps` | int32 | burst rate (min 2) |

### Result / `.lri` layout side

| tag | type | meaning |
| --- | --- | --- |
| `light_raw.csid` | int32 | sensor/module id inside the `.lri` |
| `light_raw.offset` | int32 | frame offset |
| `light_raw.calib_offset` / `calib_size` | int32 | calibration block location |
| `light_raw.frame_size_ab` / `_bc` / `_c` | int32 | **capture-set frame sizes** — see §2 |
| `light_raw.focal_length` | float | module focal length |

The app only ever used 7 of these (the control tags minus `stacked_capture_fw`
and `tripod_fw`). The `light_raw.*` group is the HAL's own description of the
container it writes.

---

## 2. Independent confirmation of the wide/tele split  [obs → confirms]

`COMPATIBILITY.md` states, from `.lri` geometry, that the camera fires in two
exclusive module sets: **wide** = A+B (ref A1) below ~66 mm, **tele** = B+C
(ref B4) at 71 mm and above.

The HAL exposes exactly this structure from the other side: the layout tags are
`frame_size_ab`, `frame_size_bc`, `frame_size_c` — the camera groups its frames
as **A+B**, **B+C**, and **C**. Two unrelated sources (parsed `.lri` geometry vs.
live vendor-tag descriptor) land on the same partition. Per the project's own
"an invariant beats a threshold" rule, this is the kind of agreement worth more
than either observation alone.

Note B appears in both `ab` and `bc`: the B row is shared between the wide and
tele sets, consistent with B being the hinge of the two-set scheme. **[inf]**

---

## 3. Capture pipeline, app side  [obs]

How a shot is assembled (all reversed; base classes still smali, ~67 classes
migrated to Java in `openlight-camera`):

```
UI mode (Auto/Manual/ISO/Shutter/Video)
  → CaptureRequestManager        strategy dispatch, EnumMap<Mode, ModeReqMgr>
  → ModeReqMgr                    3A (CONTROL_MODE, AE/AWB, TONEMAP=HQ), ISO/shutter
                                  from prefs, zoom (focal length + SCALER_CROP_REGION)
  → CaptureRequestBuilder        thin static helper
  → co.light.* vendor keys       via reflected hidden Key ctor  → HAL
```

Points that matter for fusion/extraction:

- **`IS_LIGHT` fork** (`CameraApp.isLight()`): on real hardware `startCapture()`
  calls `CameraManager.triggerCaptureToHal()` and the HAL does the multi-module
  stacking; off-hardware it falls back to a stock camera2 AE precapture. The
  16-module magic is entirely below the app. **[obs]**
- **transfer / fetch request** (`ModeReqMgr.setTransferRequest`): a distinct
  request type that sets `LENS_FOCAL_LENGTH`, nulls the crop region and applies
  exposure compensation — the mechanism the app uses to pull an already-captured
  frame for a given focal length out of a stacked capture. Plausibly relevant to
  how per-module frames end up in the `.lri`. **[inf]**
- **stacked capture** is a byte flag (`stacked_capture_state`), set false for
  single-frame modes and true for the fused path.

---

## 4. Hardware / platform constraints  [obs]

Decisive for the "revive it / rewrite it" question, and for what a replacement
app could even do on this hardware:

- **`Number of camera devices: 1`.** The 16 modules are not 16 camera2 devices;
  to camera2 it is one logical camera. SoC is **msm8996 (Snapdragon 820)**, no
  Treble, camera HAL blob `camera.msm8996.so` (Qualcomm QTI HAL with the
  `co.light` section layered on; `org.codeaurora.qcamera3.*` tags also present).
- **No `libcamera2ndk.so`** (NDK Camera2 arrived in API 24; device is API 23).
  Any capture path is bound to the **Java** camera2 API — there is no native/Rust
  capture route on this unit.
- **WebView is ancient AOSP** (`com.android.webview`, no updates) and no Treble,
  so an on-device Tauri/WebView UI is not realistic here. A modern desktop app
  (the existing `lumen`) is the sane target; the camera stays a capture appliance.

---

## 5. The native engine  [obs — already known here]

For completeness and cross-reference: the on-device fusion/processing engine is
**not** in the camera app — it is bundled in `light.co.lightgallery` as
`libcp.so` (namespace `CIAPI`, ~158 exported fns: `DepthEditor`, `Renderer`,
`ImagePyramid`, `ApplyTuning`, …), with `libceres.so` (Ceres Solver) and
`liblricompression.so` (`ltCompress::CompressLRI(...)`, two overloads:
string-paths and istream/ostream, `(…, int, bool)`).

These libraries are **already vendored in this repo**
(`vendor/light-l16/APKs/Firmware-1.3.5.1/`, including a `libcp.dylib`); this
section adds nothing to the fusion side. Its value is only the confirmation that
`libcp` strings expose the `ltpb` calibration schema — `MirrorSystem`,
`MirrorActuatorMapping`, `GeometricCalibration`, `RefinedGeomCalib`,
`ToFCalibration` — which corroborates the mirror-pose geometry in
`OPEN-QUESTIONS.md` from the engine's own symbol table. Build path in the binary:
`…/00WW-1.3.5.1/light/compimaging/camera/protobuf/…`.

---

## 6. Cross-check summary

| our capture-side model | vs. lri-rs model | verdict |
| --- | --- | --- |
| 16 modules, A/B/C = 28/70/150 mm | same (`CameraID`) | **matched** |
| wide=A+B, tele=B+C exclusive sets | COMPATIBILITY known-good | **confirmed, independent source** (§2) |
| movable mirrors on B/C | mirror-pose geometry | **confirmed** via `libcp` symbols (§5) |
| "all 16 fire at once" | — | **corrected**: focal-dependent subset (~10), never all 16 |
| fusion in a separate service | — | **corrected**: native engine is in the gallery APK (§5) |

## Caveats

Single unit, single firmware (1.3.5.1) — the same n=1 that everything else here
rests on. The vendor-tag descriptor is a property of this HAL build; a different
firmware could differ, which is exactly the kind of row `COMPATIBILITY.md` wants.
