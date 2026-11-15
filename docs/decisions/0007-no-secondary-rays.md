# 0007: Realistic images use direct shading only

2026-10-31. Accepted. Removes the "extras" of 2026-10-14.

## Context

Realistic images had optional secondary rays: shadows, ambient-occlusion rays,
reflections, see-through water, and noise with an à-trous filter. They were traced
on the CPU, with a GPU tracer of their own (`shading_tracer`) and still frames in
the viewer. They cost several seconds a frame (8 AO rays: 3 s at 800 × 900 on 96
threads) and a tracer interface through three crates, for pictures that served no
geometry question.

## Decision

Keep the material looks (each `Mn` card sorted by composition, `Looks`) and direct
physically based shading:

- GGX and Lambert, under a key and a fill light;
- a sky-to-ground environment;
- ACES tone mapping and sRGB.

Shading uses each pixel's hit only (`llevant_render::shading`; the viewer's
`colour.wgsl` mirrors it in f32). The secondary rays, their tracer interfaces and
the still frames are removed, from the CLI (`--extras`), Python (`extras=`) and
the viewer.

## Consequences

- A realistic image costs what a flat one costs: the colour pass adds 0.9 ms at
  1080p.
- Depth cues come from the viewer's screen-space ambient occlusion and outlines.
- About 2 000 lines are gone, with the GPU code path that only they used.
