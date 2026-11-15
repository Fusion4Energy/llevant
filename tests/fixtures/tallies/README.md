# Tally fixtures

MCNP output used by the field tests (`llevant-scene/tests/tallies.rs`, the CLI,
viewer and Python tests), copied from
[aci-tallies](https://github.com/AlvaroCubi/aci-tallies)' `tests/fixtures` (v0.1.0),
which reads them and has the full set:

- `pencil_beam_input.mcnp`, `.msht`: a pencil beam through FMESH tallies (`TR=` included),
  every voxel the beam crosses scoring its chord over its volume;
- `mctal/probe`: one ray along +x through slabs and a lattice, so every F4 bin is its
  chord over its volume; `mctal/levels` and `mctal/kcode`: other MCTAL layouts;
- `d1suned/`: D1SUNED cell-under-voxel meshes (`out=cuv`), with their `README.md`.

Each `.m` is the MCTAL file of the input with the same stem.
