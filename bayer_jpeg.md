# Bayer JPEG (BJPG)

Bayer JPEG is a proprietary container format used by the **Light L16** for some RAW captures. Notes for the **isamarin × IGRS** fork. Instead of writing packed 10-bit sensor data, the camera stores one or more **8-bit JPEG** images inside a `BJPG` wrapper.

We still do not know **when** the firmware chooses Bayer JPEG over `RAW_PACKED_10BPP`, or what triggers the switch.

## Where it appears in LRI

Inside a `CameraModule.sensor_data_surface`:

| `format` proto value | Name | This fork |
| -------------------- | ---- | --------- |
| `RAW_BAYER_JPEG` (0) | Bayer JPEG | `DataFormat::BayerJpeg` |
| `RAW_PACKED_10BPP` (7) | Packed 10 bpp | `DataFormat::Packed10bpp` |

When format is Bayer JPEG:

- `row_stride` is **0**
- `data_offset` points to the start of the **BJPG header** (absolute offset from block start)
- Image dimensions in `size` are still the full sensor resolution (e.g. 4160×3120 for AR1335)

Parsing: [`lri-rs/src/block.rs`](lri-rs/src/block.rs) (`DataFormat::BayerJpeg` branch).

## BJPG header layout

All integers are **little-endian**. Total fixed header: **1576 bytes**.

| Offset | Size | Type | Field |
| ------ | ---- | ---- | ----- |
| 0 | 4 | ASCII | Magic `"BJPG"` |
| 4 | 4 | u32 | `format_type` |
| 8 | 4 | u32 | `jpeg0_len` |
| 12 | 4 | u32 | `jpeg1_len` |
| 16 | 4 | u32 | `jpeg2_len` |
| 20 | 4 | u32 | `jpeg3_len` |
| 24 | 1552 | — | Unknown / padding |
| **1576** | — | — | Start of `jpeg0` data |

```
data_offset
    │
    ▼
┌────────┬────────────┬──────────────────────────────────┬─────────┬─────────┬ ...
│ "BJPG" │ format_type│ jpeg0_len │ jpeg1_len │ ... │ 1552 unknown │ jpeg0 │ jpeg1 │ ...
└────────┴────────────┴──────────────────────────────────┴─────────┴─────────┴ ...
│◄────────────── 1576 bytes ──────────────────────────────►│
```

### `format_type`

| Value | Mode | JPEG blobs used |
| ----- | ---- | --------------- |
| `0` | Colour (Bayer) | `jpeg0`, `jpeg1`, `jpeg2`, `jpeg3` |
| `1` | Monochrome | `jpeg0` only (`jpeg1..3` lengths ignored) |

### Payload size

```
total_bytes = 1576 + jpeg0_len [+ jpeg1_len + jpeg2_len + jpeg3_len if format_type == 0]
```

The parser does not validate that this fits inside the block.

## Colour mode (`format_type = 0`)

The AR1335 Bayer frame is split into **four half-resolution JPEGs** — one sub-image per 2×2 Bayer phase (conceptually: one JPEG per colour position in the tile).

Each JPEG decodes to `(width/2) × (height/2)` grayscale samples. The four planes must be **interleaved** back into a full-resolution Bayer mosaic.

### Colour plane reassembly (`lri-rs`)

[`lri-rs/src/bayer_jpeg.rs`](lri-rs/src/bayer_jpeg.rs) decodes each JPEG with `zune_jpeg` and interleaves half-res samples:

```
bayer_x = (in_x * 2) + (offset % 2)
bayer_y = (in_y * 2) + (offset / 2)
```

JPEG index → offset: `jpeg0→0`, `jpeg1→1`, `jpeg2→2`, `jpeg3→3`.

The correct mapping of JPEG order to R/G/B positions is **not fully confirmed**.

### 10-bit precision via splitting

JPEG is limited to **8 bits per sample**, but the AR1335 outputs **10-bit** RAW. The L16 workaround (hypothesis):

- Split the frame into four spatially offset (or bit-split) sub-images
- Store each as 8-bit JPEG
- **Sum** the four planes on decode to recover extended dynamic range

This matches field observations and the comment in the original notes. The exact bit-reconstruction formula is **not implemented** in this fork — the experimental path only interleaves decoded 8-bit values.

## Monochrome mode (`format_type = 1`)

`jpeg0` is a single **full-resolution grayscale JPEG** (`width × height`). Used for `SENSOR_AR1335_MONO` modules.

## Implementation status (this fork)

| Layer | Status |
| ----- | ------ |
| `lri-rs` — BJPG header parse | Done — `RawData::BayerJpeg` |
| `lri-rs` — JPEG decode | Done — [`bayer_jpeg.rs`](lri-rs/src/bayer_jpeg.rs) |
| `RawImage::decode_pixels()` | Both Packed10bpp and Bayer JPEG |
| `RawImage::unpack()` | Packed10bpp only (legacy helper) |
| `light extract` | Exports both formats to PNG |

8-bit JPEG samples are promoted to 10-bit range with `(byte as u16) << 2`. Full bit-plane summing is not implemented.

### Next steps for L16 prep

1. Confirm JPEG index → Bayer phase mapping against known CFA + `sbro`
2. Investigate 10-bit reconstruction (sum vs bit-plane split)
3. Unit tests against `.lri` files with both formats (`light gather` to survey a directory)

## Detecting Bayer JPEG in a folder

```bash
cargo run --release -p light -- gather /path/to/photos/
```

Per-camera sensor codes in output:

- **Cyan** (`a13`, `a1m`) — Bayer JPEG
- **Yellow** — Packed 10 bpp

## See also

- [LRI.md](LRI.md) — block structure, `data_offset`, `CameraModule`
- [`lri-rs/src/block.rs`](lri-rs/src/block.rs) — parser reference implementation