# Compatibility registry

Which cameras this has actually been run on, and what happened. Extend it by
opening a pull request or an issue with your row — you do not need to ask.

Everything here rests on a very small sample. Every geometry conclusion in
[OPEN-QUESTIONS.md](OPEN-QUESTIONS.md) was derived from **one camera**, and twice
already a rule inferred from it turned out to be an artifact of the captures it
was inferred from. More cameras is not a nice-to-have; it is the only thing that
can tell a real property of the L16 from a property of this particular unit.

## No personal data

Firmware and hardware behaviour only. No names, no emails, no owner column, no
serial numbers, no capture locations. If you want to be credited, say so in the
pull request and it goes in the commit, not in this table.

Sample captures are welcome and are the single most useful thing you can
contribute — but check them first:

```bash
cargo run --release -p light --example privacy_scan -- /path/to/*.lri
```

It reports whether a capture carries a GPS fix, and groups locations to ~1 km so
the answer does not itself leak a position. Captures from this project's own
camera carry none, but that is a property of those files, not of the format.

## Registry

| firmware | modules OK | extract | fusion | notes |
| --- | --- | --- | --- | --- |
| 1.3.5.1 | 16 / 16 | works, 61 captures | partial | reference unit for all development |

**extract** — per-module RAW → DNG. This is the part that is expected to work.
**fusion** — the 16→1 combine. "partial" is honest: see FUSION.md.

## What is worth reporting

- Firmware version that differs from 1.3.5.1 — most valuable of all, since every
  parsing assumption here was read off a single firmware.
- A module that decodes wrong, or does not appear at all.
- A capture that fails to parse. Please attach it if you can; a file that breaks
  the parser is more useful than ten that do not.
- Anything where the DNGs come out but look wrong — orientation, colour, black
  level.

## Known-good behaviour to compare against

Reported by the reference unit on firmware 1.3.5.1, so you can tell "different
camera" from "broken":

- The camera fires in two exclusive module sets: **wide** (A1–A5 + B1–B5,
  reference A1) below ~66 mm, **tele** (B1–B5 + C1–C6, reference B4) at 71 mm and
  above. Never both. A capture that mixes them would be a genuinely new finding.
  Independently confirmed from the HAL side: the `co.light` vendor tags name the
  frame sizes `frame_size_ab` / `frame_size_bc` / `frame_size_c` — the same
  A+B / B+C / C partition, read off the live device rather than the `.lri`. See
  [HAL-CAPTURE.md](HAL-CAPTURE.md).
- A row is 28 mm, B row 70 mm, C row 150 mm equivalent.
- C-row modules are monochrome.
