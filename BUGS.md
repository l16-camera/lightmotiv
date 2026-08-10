# Bugs and why they happened

Companion to [FUSION.md](FUSION.md). That file records what we *learned*; this one
records what we got *wrong*, and — more usefully — **what let the mistake survive**.

Most bugs here share one shape: the code produced plausible output, so nothing
screamed. In a pipeline whose ground truth is a dead company's binary, "looks
about right" is not evidence. Each entry therefore ends with a **guard**: the
check that would have caught it, or now does.

## Entry template

```markdown
### YYYY-MM-DD — Short title

**Where:** `path/to/file.rs` · fixed in `<sha>`
**Symptom:** what you actually observe
**Root cause:** the real mechanism
**Why it hid:** what made it survive review and tests
**Guard:** the check that catches a recurrence
```

---

## Log

### 2026-08-10 — DESKTOP profile silently rendered at 1/16 scale

**Where:** `tools/libcp-export/libcp_export.cpp` (`try_dump` / `poll_until`) · fixed in `f65ad63`
**Symptom:** `light libcp --profile 3` asked for `10432×7824` and wrote a `652×489` JPEG.
Exit code 0, no warning, and the run finished in 3.4 s.

**Root cause:** libcp fills its output `ImagePyramid` **coarse-to-fine**. `try_dump`
scanned every level and accepted the first one carrying signal, then returned
immediately. Early in a render only the coarsest level (L4) has data, so L4 always won.

**Why it hid:** three things lined up.
1. Profile 1 (13 MP) was unaffected — L0 fills inside a single polling interval — and
   profile 1 is what everything was tested on.
2. The output was a *correct image*, just small. Nothing looked broken.
3. The M4 smoke test in `LUMEN_PLAN.md` asserted the **depth map** dimensions
   (320×240, genuine) but never asserted the **image** dimensions.
4. Log ordering hid it: `SUCCESS wrote … (652×489)` goes to stdout, the per-level
   `wait`/`ACCEPT` lines to stderr. Buffering interleaved them so the SUCCESS line
   appeared *before* the level scan, reading as "rendered, then polled".

**Blast radius:** every DESKTOP-profile output produced before this fix is downscaled.
A golden set built on the old helper would have baked in wrong ground truth — which is
exactly what it exists to prevent.

**Guard:** assert output dimensions against the requested ROI for every profile, not
just depth-map dimensions. A render that returns fewer pixels than requested must fail
loudly or announce itself (`DEGRADED 1` on stdout, `WARNING` on stderr).

---

### 2026-07-21 — Every mirror module's pose was an improper rotation

**Where:** `lri-rs/src/mirror_pose.rs` · fixed in `f5f6297`
**Symptom:** B1, B5, C1, C3 reconstructed reflected on every capture and every focal
length. C1's NCC was negative wherever it fired.

**Root cause:** `reflection_matrix` is a Householder matrix, so `det = −1` always, and
`r_cam` is proper — every composed mirror pose was therefore improper. `flip_x_mat`
negates a row and restores `det = +1`, but it was applied **only when
`flip_img_around_x` was true**, leaving the other modules reflected.

**Why it hid:** the correction existed and worked, so the modules where the flag was
true looked right — and those were the ones being examined. The flag reads like a
mounting detail, which makes "apply it when set" look obviously correct.

**Guard:** a camera extrinsic lives in SO(3). Assert `det(R) = +1` for all 16 modules
across several captures — no per-module mounting flag can make an improper matrix an
acceptable pose. The parity signature is unmistakable once measured: NCC moves on
exactly the modules where `flip = false`, and nowhere else.

---

### 2026-07-20 — Planar homography parallax term had the wrong sign

**Where:** `lri-rs/src/warp.rs` · fixed in `8ca1dbe`
**Symptom:** fused output was noise rather than a scene.

**Root cause:** the parallax term in the planar homography was added where it should
have been subtracted.

**Why it hid:** the existing test only covered **depth → ∞**, where the parallax term
vanishes entirely. The test passed for both signs.

**Guard:** any test of a depth-dependent transform must include a **finite** depth. A
test at the limit where a term disappears cannot constrain that term's sign.

---

### 2026-07-20 — Bayer preview decimated instead of debayered

**Where:** `light/src/thumbnail.rs` · fixed in `8ca1dbe`
**Symptom:** single-module fusion previews were noise.

**Root cause:** the preview subsampled the Bayer mosaic by naive decimation, so every
sampled pixel landed on a single CFA channel — one colour plane plus aliasing.

**Why it hid:** it was "just a preview", so it was never held to the standard of the
export path. But the preview *was* how fusion quality was being judged by eye, which
made it silently authoritative.

**Guard:** box-average each tile into luma rather than point-sampling a mosaic. More
generally: if a preview is what you use to judge correctness, it is not a preview — it
needs the same scrutiny as the output.

---

### 2026-08-10 — OPEN: readiness is judged by non-zero pixel share, costing ~10 min per render

**Where:** `tools/libcp-export/libcp_export.cpp` (`buffer_has_signal`) · **not fixed yet**
**Symptom:** `--profile 3` takes 9 min 52 s per capture. At 101 captures that is ~17 h for
a golden set.

**Root cause:** `poll_until` accepts level 0 once the sampled share of non-zero pixels
crosses a hardcoded `need` (0.55 above 20 MP). That share is not a completion signal.
Measured on `L16_00064`: full-res L0 was present at **poll 4** (50.6 % non-zero,
88.7 % in the centre), then the buffer stayed **byte-identical for ~340 polls** before
creeping 52.6 → 53.8 → 55.1 % and finally tripping the threshold at poll 349. The poll
step caps at 2 s, so those wasted rounds are the entire cost: 82 s of CPU across 592 s
of wall clock — 86 % of the run was spent asleep.

**Why it is dangerous beyond slowness:** the threshold was tuned against one bright
frame. A capture with a large dark region legitimately never reaches 55 % non-zero, so
it will burn the full 180 s budget and then fall through to `degraded_dump()` — writing
a downscaled image with a warning nobody reads during a 17-hour batch. The 1/16 bug is
fixed; this would reintroduce its effect through a different door.

**Proposed fix:** judge **stability**, not fill. Accept L0 when the sampled signature is
unchanged across N consecutive polls and coverage exceeds a low floor, keeping an
absolute timeout as a backstop. On the measured data that fires around poll 7 — seconds
instead of minutes, and it is scene-independent.

**Guard (once fixed):** a render whose wall time is orders of magnitude above its CPU
time is waiting on a bad predicate, not computing. Assert the ratio in the runner.

---

## Recurring patterns

Worth re-reading before declaring anything "works":

1. **Tested only on the easy configuration.** Profile 1 hid the pyramid bug; the
   `flip = true` modules hid the parity bug. Whatever is convenient to test is the
   thing that will not exhibit the fault.
2. **Tested at the limit where the term vanishes.** The infinity-depth test could not
   see the parallax sign.
3. **Plausible output read as correct output.** A small image, a reflected module, a
   noisy preview — all shipped because nothing crashed.
4. **The judging instrument was itself unverified.** Fusion quality was judged through
   a preview that was wrong.
5. **Silent degradation.** Every one of these returned exit code 0. Degradation must be
   announced in-band, not inferred from dimensions after the fact.
