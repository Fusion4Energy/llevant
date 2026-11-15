# 0005: GPU rays are traced in short slices

2026-10-28. Accepted.

## Context

The viewer lost its window device on the reference model with only the copper
cells shown: two runs in four failed. Two causes combined:

- **Long dispatches.** Single tiles took up to 0.9 s, bounded by the latency of
  their slowest rays, not by how many rays they held.
- **wgpu's frame timeout.** On Windows, wgpu-hal 30 waits for a swapchain image
  with a 1 s fence timeout and turns that wait's `VK_TIMEOUT` into a lost device.
  A window frame queued behind trace work for more than a second therefore loses
  the window.

It was neither a driver reset (TDR) nor a lack of memory.

## Decision

Rays are traced in slices (`llevant_gpu::Job`).

- **Blocks.** Rays go in blocks of 64 (8 × 8 pixels), one block per workgroup.
- **Pausing.** In each slice a ray walks at most `slice_work` program words
  (default 256), then pauses:
  - its state (20 words) goes to its lane's slot;
  - on resuming, it locates itself again from level 0, which does not count
    against its `max_work`.
- **Lanes.** A ray keeps its lane from slice to slice.
- **Dispatches.** A slice runs only the workgroups with rays left, through an
  indirect dispatch.
- **Next block.** A workgroup whose rays have all ended takes the job's next
  block.
- **In the viewer,** every slice stays this short (`beside_window`), so the
  window's frames wait at most a few milliseconds behind tracing.

## Evidence

- **Between devices.** A tiny job on another device waited 87 ms (median) behind
  512 ms dispatches, against 2 ms behind 4 ms dispatches, at no cost in
  throughput. The driver switches devices at dispatch boundaries.
- **Viewer, copper isolated.** Orbit frames went from 23–78 ms median and up to
  865 ms, to 16.8 ms median, 27–30 ms at the 95th percentile and 108 ms at most.
  No run lost a device.
- **Whole frames.** As fast as one dispatch for solid views (+2 %), and 25–40 %
  faster where hidden cells make rays long.

## What did not work

- **Compacting paused rays into fewer warps.** Warps then mixed rays from all over
  the image and ran their lanes' paths one after another: 900 ms against 305 ms.
- **Counting the relocation of resumed rays against `max_work`.** Some rays then
  gave up where one dispatch finished them.
