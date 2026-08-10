# Свой Lumen (Luminat) — план

**Статус:** M1 **landed (code)** 2026-08-05 — UI smoke: run `./target/release/lumen`  
**Репо:** `/Users/igor/IGRS/luminat`  
**Журнал Grok:** openlight-camera `GROK.RESEARCHES.md` § Plan Lumen

## Определение done

Открыл `.lri` → **цветная** склейка Light-quality → export JPEG → без Qt-Lumen и без CLI.

## Архитектура (зафиксировано)

| Слой | Выбор |
| --- | --- |
| Quality 16→1 | **libcp / CIAPI** only |
| UI | **Tauri `lumen`** (product shell) |
| Software fuse | experimental / lab |
| libcp dylib | from Lumen.app, not redistributed |
| Camera APK | Claude, parallel |

## Вехи

| ID | Цель | Критерий | Status |
| --- | --- | --- | --- |
| **M0** | CLI foundation | `light libcp` 1/3, extract, drop | **done** |
| **M1** | libcp-first app | folder + View p1/p3 + export JPEG + fuse hidden | **done** |
| **M2** | camera + batch | adb library, cache, multi-export | **done** |
| **M3** | ship | .app package, first-run setup, zoom/pan | **done** |
| **M4** | Lumen-like extras | refocus/depth if CIAPI allows | **done** (code) 2026-08-05 |

## M1 checklist

- [x] Plan written
- [x] `libcp_status` pill in app
- [x] Viewer panel primary: **Render** (Light engine)
- [x] Profile 1 (13 MP) / Profile 3 (Desktop canvas)
- [x] Cache beside LRI: `<stem>.libcp.p{N}.jpg`
- [x] Export JPEG / Show file
- [x] Software fuse under «Experimental»
- [x] Library sidebar (folder scan) remains
- [x] `cargo build --release -p lumen`

## Anti-goals

- Fix official Lumen Qt
- Open-source full SR before product
- 52 MP DNG in Lightroom v1
- Software fuse as default renderer

## M2 checklist

- [x] `light::camera` (adb status / list / pull)
- [x] lumen: camera pill + modal multi-select pull
- [x] optional render-after-pull
- [x] batch_libcp on library list
- [x] cache beside LRI (`*.libcp.pN.jpg`) + camera pull cache
- [x] `cargo build --release -p lumen`

## M3 checklist

- [x] Setup wizard (libcp dir + helper) → `~/Library/Application Support/Luminat/config.json`
- [x] Zoom/pan viewer (wheel, drag, Fit/100%/±)
- [x] `make package-macos` → `dist/Luminat.app` (embeds libcp-export)
- [x] Open wizard if libcp missing on launch / Render

```bash
make package-macos
open dist/Luminat.app
```

## M4 checklist

- [x] RE: `setProperty(ParamFloat)` — FocusDepth=1, FNumber=3 (gallery JNI)
- [x] RE: `DepthEditor` shell = 16 B shared_ptr; `getDepthAtPoint(Point<float>)`
- [x] `libcp-export`: fnumber / focus_depth / fx,fy / depth.ppm
- [x] `light libcp --fnumber --focus-depth --focus-x/y --depth-map`
- [x] lumen UI: aperture slider, Alt+click / Click focus, depth thumb + toggle
- [x] DOF-aware cache names (`*.libcp.pN_f…_xy….jpg`)
- [x] smoke L16_00026: f/2.5 @ p1 (4160×3120); f/4 + click (0.5,0.4) + depth @ p3 → z≈16829 mm, depth 320×240 valid

```bash
make libcp-export
cargo build --release -p light -p lumen
light libcp --lri /path/photo.lri -o /tmp/m4 --fnumber 4 --focus-x 0.5 --focus-y 0.4 --depth-map
```

## Next after M4

Optional notarized dmg; more CIAPI props (exposure/WB) if needed.
