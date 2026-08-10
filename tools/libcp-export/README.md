# libcp-export

Optional x86_64 helper that runs Light’s closed fusion engine (`libcp.dylib` / CIAPI)
and writes a fused RGB PPM. Used by:

```bash
light libcp --lri photo.lri -o ./out
```

## Why a separate binary?

- `libcp.dylib` from Lumen is **x86_64 only** (Rosetta on Apple Silicon).
- luminat `light` is native arm64 — it **spawns** this helper under `arch -x86_64`.
- We do **not** ship proprietary dylibs in the repo.

## Obtain libcp

Copy from an installed Lumen app (or extract from Lumen.dmg):

```text
Lumen.app/Contents/Frameworks/libcp.dylib
Lumen.app/Contents/Frameworks/libceres.dylib
# optional, some Lumen builds:
Lumen.app/Contents/Frameworks/liblricompression.dylib
```

Either leave them in the app, or drop into `tools/libcp-export/vendor/`.

## Build helper

```bash
make libcp-export
# → tools/libcp-export/libcp-export  (Mach-O x86_64)
```

Requires Xcode clang, Rosetta (`softwareupdate --install-rosetta` once).

## Run via light (preferred)

```bash
# auto-find Lumen.app or vendor/
export LUMINAT_LIBCP_DIR="/Applications/Lumen.app/Contents/Frameworks"
./target/release/light libcp --lri /path/photo.lri -o /tmp/out --format jpg

# or point at helper + dylibs explicitly
export LUMINAT_LIBCP_EXPORT="$PWD/tools/libcp-export/libcp-export"
export LUMINAT_LIBCP_DIR=/path/to/dir/with/libcp.dylib
```

## Run helper directly

```bash
arch -x86_64 ./tools/libcp-export/libcp-export \
  /Applications/Lumen.app/Contents/Frameworks/libcp.dylib \
  photo.lri /tmp/out.ppm 1
```

### DOF / refocus / depth (M4)

```text
libcp-export libcp.dylib in.lri out.ppm [profile] \
  [fnumber=-1] [focus_depth_mm=-1] [fx=-1] [fy=-1] [depth.ppm]
```

| Arg | Meaning |
| --- | --- |
| `fnumber` | 2–15 → `ParamFloat(3)` ViewDofFNumber; ≤0 leave default |
| `focus_depth_mm` | >0 → `ParamFloat(1)` ViewDofFocusDepth |
| `fx`,`fy` | [0,1]² click; after first render `DepthEditor::getDepthAtPoint` → set focus → re-render |
| `depth.ppm` | optional path; 320×240 depth colormap |

Via `light`:

```bash
light libcp --lri photo.lri -o ./out --fnumber 4 --focus-x 0.5 --focus-y 0.4 --depth-map
light libcp --lri photo.lri -o ./out --focus-depth 2500 --fnumber 2.5
```

### RendererProfile (CIAPI int)

From camera `ProcessRequest.ProcessingProfile.mProfileNumber` (smali):

| int | Name | Measured L0 canvas (L16_00026) | Notes |
| --- | --- | --- | --- |
| 0 | THUMBNAIL | ~520×390 pyramid | preview; full export flaky |
| **1** | **MOBILE** | **4160×3120 (~13 MP)** | default; fast ~2 s |
| 2 | CAMERA | 4160×3120 (~13 MP) | on-device “13 MP” tier |
| **3** | **DESKTOP** | **10432×7824 (~81.6 MP)** | Lumen canvas; SR tier ~11 s |

Marketing **52 MP** is not a separate profile — DESKTOP builds the full Lumen canvas
(`ViewOutput::LUMEN_CANVAS`); export crop / tone may advertise ~52 MP. Super-res does
**not** include mono modules (libcp strings).

```bash
light libcp --lri photo.lri -o ./out --profile 1   # 13 MP, fast
light libcp --lri photo.lri -o ./out --profile 3   # desktop canvas, quality
```

## Env

| Variable | Meaning |
| -------- | ------- |
| `LUMINAT_LIBCP_DIR` | Directory containing `libcp.dylib` (+ ceres) |
| `LUMINAT_LIBCP` | Full path to `libcp.dylib` |
| `LUMINAT_LIBCP_EXPORT` | Full path to this helper binary |
| `LIBCP_MAX_WAIT_MS` | Poll budget (default 90000) |

## Provenance

Port of Grok A1 harness (`a1_export.cpp`) from openlight-camera research
(`GROK.RESEARCHES.md` A1/B2). CIAPI mangled symbols, ROI layout, async poll.
