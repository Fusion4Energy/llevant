# Python API

Everything in the `llevant` module. For a guided introduction, read
[the Python guide](../guide/python.md) first.

A few rules hold throughout:

- **Numbers.** Cells, surfaces, materials and universes are MCNP numbers; lengths
  are in cm.
- **Batches.** Functions that take many points or lines take anything
  `numpy.asarray` accepts, of shape `(N, 3)`, and return numpy columns.
- **Errors.** `OSError` for files, `ValueError` for bad arguments, `LlevantError`
  for other failures.

::: llevant
