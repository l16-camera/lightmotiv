# lightmotiv

Rust workspace for **Light L16** `.lri` (Light Raw Image) files — parse, survey, export per-camera RAW, and research the Lumen 16→1 fusion pipeline.

> **Built on [gennyble/lri-rs](https://github.com/gennyble/lri-rs).** lightmotiv is a
> fork of that project, which reverse-engineered the `.lri` container and the Protobuf
> metadata inside it. Every `.lri` file this workspace opens is opened by gennyble's
> parser. What the fork adds on top is the `light` CLI with per-module Adobe DNG
> output, the Lumen desktop GUI, and camera pull over `adb`. See [NOTICE](NOTICE) for
> the file-by-file split and the licensing status of the upstream parts.

Maintained by **isamarin × IGRS**. Version: **CalVer** (`YYYY.M.D`) — see `VERSION` and `./scripts/calver`.

## Quick start

```bash
# Optimized release build (LTO + native CPU flags on Apple Silicon)
make release

# Survey a folder of captures (includes fusion metadata summary)
./target/release/light gather /path/to/photos/

# Extract all camera modules to DNG (parallel, mmap-backed)
./target/release/light extract photo.lri ./output/ --jobs 8
```

Install globally:

```bash
make install   # → ~/.cargo/bin/light
```

### Desktop GUI (Tauri 2)

```bash
make lumen-release
./target/release/lumen

# shippable app (embeds libcp-export)
make package-macos
open dist/Luminat.app
```

Drag-drop `.lri`, camera grid, **libcp Render** (Light engine, profile 13 MP / Desktop), DNG export. Software fuse is under Experimental. See [LUMEN_PLAN.md](LUMEN_PLAN.md).

For live reload during UI work, install the Tauri CLI once (`cargo install tauri-cli`) and run `cargo tauri dev` from `lumen/src-tauri`.

### Camera + batch (M2, in Luminat app)

- **From camera…** — adb list/pull `/sdcard/DCIM/Camera/*.lri` (optional libcp after pull)
- **Batch render** — libcp all files in the library sidebar
- Cache: `~/Library/Caches/lri-drop/camera/` and `*.libcp.p{N}.jpg` beside each LRI

## `light` CLI

| Command | Description |
| ------- | ----------- |
| `light gather <dir>` | Metadata + fusion + **mono** tags for every `.lri` |
| `light extract <lri\|dir> <out>` | Per-camera DNG; dir → batch; `--only-mono`, mono PNG previews |
| `light fuse --lri … -o …` | Software plane-sweep fusion (research / geometry lab) |
| `light libcp --lri … -o …` | Light `libcp.dylib` quality fuse (needs Lumen frameworks) |
| `light libcp --dir … -o …` | **Batch** libcp over a folder of captures |
| `light libcp … --fnumber 4 --focus-x 0.5 --focus-y 0.4 --depth-map` | **M4** aperture + click-refocus + depth (DESKTOP) |

`gather` appends fusion hints and mono, e.g. `a1m … \| mono:A2≈28mm fus geo:16/16`.

### Mono (A2 / C6)

Panchromatic AR1335 modules export as `A2_mono.dng` / `C6_mono.dng` plus optional
`mono/*.png` previews and `mono.json`. GUI: mono panel + “Export mono DNGs”.

```bash
./target/release/light extract photo.lri ./out --only-mono
./target/release/light extract ./forest/ ./dngs/          # batch per stem
```

### Optional libcp backend (macOS + Rosetta)

Native `light fuse` is open-source MVP fusion. For Lumen-quality RGB, use the closed
engine from Lumen.app (not redistributed):

```bash
make libcp-export          # build x86_64 tools/libcp-export/libcp-export
make release
# libcp.dylib + libceres.dylib from Lumen.app/Contents/Frameworks
export LUMINAT_LIBCP_DIR="/Applications/Lumen.app/Contents/Frameworks"
./target/release/light libcp --lri photo.lri -o ./out --format jpg          # profile 1 ≈ 13 MP
./target/release/light libcp --lri photo.lri -o ./out --profile 3 --format jpg  # DESKTOP 10432×7824
./target/release/light libcp --lri photo.lri -o ./out --fnumber 4 \
  --focus-x 0.5 --focus-y 0.4 --depth-map   # refocus + depth (auto profile 3)

./target/release/light libcp --dir ./forest/ -o ./libcp-out --format jpg
```

Details: [tools/libcp-export/README.md](tools/libcp-export/README.md).

Replaces the older `prism` and `lri-study` binaries (still in repo, no longer in workspace).

## Workspace

| Crate | Role |
| ----- | ---- |
| **lri-rs** | Library — `LriFile::decode()`, `RawImage::decode_pixels()`, `LriFile.fusion` |
| **lri-proto** | Protobuf types ([dllu/lri-rs](https://github.com/dllu/lri-rs) / Lumen) |
| **light** | CLI + shared lib (DNG, thumbnails, session cache) |
| **lumen** | Tauri 2 desktop GUI |

## Documentation

- [LRI.md](LRI.md) — block format, cameras, colour calibration
- [bayer_jpeg.md](bayer_jpeg.md) — BJPG container
- [FUSION.md](FUSION.md) — Lumen combine research log (geometry, depth, blend) — **living doc for humans and agents**
- [BUGS.md](BUGS.md) — bug journal: root causes and what let each mistake survive

## Library example

```rust
let data = std::fs::read("photo.lri")?;
let lri = lri_rs::LriFile::decode(&data)?;

for img in lri.images() {
    let pixels = img.decode_pixels()?; // Packed10bpp + Bayer JPEG
    let (black, white) = lri.levels_for(img.sensor);
}

// Fusion pipeline inputs (geometry, ToF, IMU, GPS)
let fusion = &lri.fusion;
println!("geometry modules: {}", fusion.geometry_module_count());
```

Via `light` session API (mmap + cached decode):

```rust
let session = light::session::LriSession::open("photo.lri")?;
session.with_lri(|lri| { /* ... */ })?;
```

## Versioning (CalVer)

| File / tool | Role |
| ----------- | ---- |
| `VERSION` | Single source of truth (`2026.8.10`) |
| `./scripts/calver` | `show`, `sync`, `check`, `bump`, `bump-micro` |
| `make version-bump` | Set today's UTC date and sync `Cargo.toml` + `tauri.conf.json` |

Same-day rebuilds use semver pre-release: `2026.7.14-dev.1`.

Release tag: `git tag v2026.7.14 && git push --tags` → GitHub Actions builds binaries.

## CI

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) on push/PR:

- CalVer consistency check
- `cargo test --workspace`
- Release build: `light` on Linux, `light` + `lumen` on macOS

[`.github/workflows/release.yml`](.github/workflows/release.yml) — artifacts on version tags.

Local checks:

```bash
make version-check
cargo test --workspace
make bench    # tenbit unpack benchmark
```

## Apple Silicon tuning

| Setting | Location |
| ------- | -------- |
| `target-cpu=native` | [`.cargo/config.toml`](.cargo/config.toml) |
| Fat LTO, 1 codegen unit | `[profile.release]` in root `Cargo.toml` |
| `release-fast` profile | Thin LTO for quicker iteration (`make release-fast`) |
| P-core thread count | `light/src/threads.rs` — `sysctl hw.perflevel0.physicalcpu` |
| Zero-copy block parse | `lri-rs` mmap / slices into input buffer |
| 10 bpp unpack | 8× unrolled (`lri-rs/src/unpack.rs`) |
| Fast grid thumbnails | Single JPEG plane + parallel batch (`light/src/thumbnail.rs`) |
| Session cache | `LriSession` — one decode per open file (`light/src/session.rs`) |
| Parallel DNG export | `rayon` in `light extract` |

## What works

| Feature | Status |
| ------- | ------ |
| Block parse with error handling | Yes |
| Packed 10 bpp unpack | Yes |
| Bayer JPEG decode → pixels (`zune-jpeg` 0.5) | Yes |
| DNG export (both RAW formats) | Yes |
| GUI thumbnails + drag-drop + export progress | Yes (`lumen`) |
| `sensor_data` black/white levels | Yes (`levels_for`) |
| Fusion metadata (geometry K/R/t, ToF, IMU, GPS) | Partial — [FUSION.md](FUSION.md) |
| 16→1 combine via Light `libcp` (CIAPI) | Yes — `light libcp`, Luminat **Render**, aperture / click-refocus / depth map |
| 16→1 **own** combine (open, in-tree) | Grayscale MVP — undistort + plane-sweep depth + warp + blend; colour pending ([FUSION.md](FUSION.md)) |

## Resources

- [`vendor/light-l16/`](vendor/light-l16/) — git submodule, [isamarin/light-l16](https://github.com/isamarin/light-l16) (L16 archive: firmware notes, Lumen app, hardware, guides)
- [FUSION.md](FUSION.md) — submodule paths and fusion research log

Clone:

```bash
git clone --recurse-submodules https://github.com/isamarin/lri-rs.git
# or after clone:
git submodule update --init
```

## Credits

- Original parser & docs — [gennyble](https://github.com/nyble) ([gennyble/lri-rs](https://github.com/gennyble/lri-rs))
- Protobuf types — [Daniel Lawrence Lu](https://github.com/dllu) ([dllu/lri-rs](https://github.com/dllu/lri-rs))
- This fork — **isamarin × IGRS**

## Licensing

Inherited crates keep their own terms, reproduced in full in each directory's
`LICENSE` file:

| Path | License | Copyright |
| ---- | ------- | --------- |
| `lri-rs/` | ISC | 2023 gennyble \<gen@nyble.dev\> |
| `prism/` | ISC | 2023 gennyble \<gen@nyble.dev\> |
| `lri-study/` | ISC | 2023 gennyble \<gen@nyble.dev\> |
| `lri-proto/` | MIT | 2021 Daniel Lawrence Lu |

Everything original to this fork — `light/`, `lumen/`, `tools/`, `scripts/`,
`FUSION.md`, `LUMEN_PLAN.md`, `BUGS.md` — is licensed under either
[Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.

See [NOTICE](NOTICE) for the full file-by-file split. Contributions are dual
licensed the same way unless you say otherwise.