# 0013: The 3D view is traced for latency, not by tiles sized to a budget

2026-11-14. Accepted. Refines [0005](0005-sliced-gpu-dispatches.md) and
[0006](0006-reprojection-gpu-only.md).

## Context

On the reference model, the GPU meters read 80–98 % "busy" while a still view
refined. Most of the GPU was in fact idle. Three measurements on the laptop (RTX 3060
Laptop, 16 threads) showed why.

- **A few rays set the time.** In one job for a whole 1024² view, 99.99 % of the
  rays end within ~90 ms. The other ~470 ms of a 560 ms frame go to ~140 rays, each
  walking 20–40 k program words through void cells with long programs (124 locate
  steps at most, so not the step cap). The meters count a running kernel as busy,
  whether 64 threads are working or 46 k.
- **Previews paid that latency at the smallest size.** The scheduler's
  least-squares cost became proportional once previews settled at one size, so it
  never learned that a bigger preview costs almost nothing more. Previews stayed at
  scale 0.07: 37×69 took 134 ms, the whole 528×985 window 287 ms.
- **Work was wasted or repeated.**
  - In the frame a preview was sent, a full-resolution tile of the same camera was
    queued behind it (two batches in flight). By the next frame it was stale, but
    nothing gave it up, and the next preview waited for it. In a 10 s orbit, stale
    tiles took 4.5 s of GPU time and previews 4.2 s.
  - Refinement went one tile per job, each paying its own slowest rays and a
    readback: 1.3–1.9× slower than one job for the image.

## Decision

- **One batch at a time, stale work given up.** A restart cancels full-resolution
  batches of older generations (`TraceBackend::cancel_full_before`). A preview in
  flight is not cancelled, because an older preview is still shown, moved to the
  camera.
- **One job per still view.** The full level is one batch, its 8×8 blocks ordered
  from the centre out. The blocks a submission finishes are read back with the next
  submission and land as pieces, so the view fills in as it is traced. The
  full-level cost model and tile sizing are gone.
- **Previews bound each ray's work.** They walk at most 2048 program words a ray
  (`GpuBackend::preview_work`). A ray that gives up takes the hit of the nearest ray
  that did not, looked for in 8 directions up to 16 preview pixels away
  (`resolve.wgsl`). Full-resolution rays keep the model's `max_work`.
- **Previews are sized from their latency.** The fixed part of the cost is the
  fastest preview since the costs were reset; the per-ray part is the slope of the
  samples about it. This needs no spread of sizes.

## Evidence

Reference model, laptop, 10 s orbit then the still view (`--orbit 10`):

| | Before | After |
|---|---|---|
| Previews per second | 12.1 | 19–21 |
| Mean preview scale | 0.07 | 0.45–0.48 |
| Still view refined | 0.65 s | 0.30–0.38 s (one 230 ms job) |
| Orbit frames, 95th percentile | 19.6 ms | 21.3–21.7 ms |

The 95th percentile of the window's frames rose by ~2 ms. The GPU is now busy with
useful work more of the time, and the window's frames wait a little longer behind it.
The UI thread itself had no frame over 8 ms while orbiting.

Work capped at 2048 words: a whole-window preview takes 40 ms instead of 287 ms, and
253 of 1 M rays give up (0.03 %). At 8192 words, 109 ms and 85 rays.

Views cut by a plane give up more, and in patches. A 264×492 preview of a close-up
cut along z: 2.05 % of rays at 2048 words (27 ms), 1.26 % at 4096 (43 ms), 0.62 % at
8192 (60 ms), 0.26 % at 16384 (87 ms), none uncapped (114 ms). Of its 2659 give-ups
at 2048 words, 1166 are more than one pixel from a ray that ended, and some 8. A
higher cap trades a little grey for much latency, so the cap stays and the fill
reaches 16 pixels; that costs nothing measurable, since only give-ups search.

The default time per preview then went from 33 ms to 66 ms, and the setting became
"While moving", responsive to sharp. Users found the highest setting smoothest, and
the orbit shows why. The window ran at the same frame rate at every setting, and
reprojection moves a preview with the camera, so steadiness comes from sharp previews.
Low-resolution ones flicker as each replaces the last. At 33 / 66 / 100 ms, previews
were at scale 0.50 / 0.84 / 0.89, 19 / 11 / 11 a second. Past ~66 ms, previews reach
their size limits.

## Consequences

- A camera job's per-block counters now start at 64 and count the rays past the
  image's edge. Before, edge blocks wrapped below zero; nothing read the counters
  until now (`tests/tiles.rs`, `blocks_done_hold_their_hits`).
- The latency floor is the fastest preview since the last reset (a new backend,
  model or visibility). If the camera turns from empty space to the model, previews
  are sized conservatively until the next reset.
- Done since in [decision 0014](0014-gpu-exit-bound.md): making the slowest rays themselves cheaper (shorter exits from void cells
  with long programs). That is what still bounds a still view's last ~200 ms.
