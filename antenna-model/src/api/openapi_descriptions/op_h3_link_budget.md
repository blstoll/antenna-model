Computes antenna gain, free-space path loss, total path loss and (optionally) G/T
for every cell of an H3 hexagonal grid laid over the Earth's surface.

The grid is centred on the H3 cell containing `feed_pointing_location` — the Earth location
the feed's beam is aimed at, **not** the feed's physical location on the antenna —
and covers `n_rings` rings around it, so the response contains
`1 + 3 * n_rings * (n_rings + 1)` cells: 1, 7, 19, 37, … up to 331 at the maximum
`n_rings` of 10.

`h3_resolution` may be supplied explicitly (0-15). When it is absent the service
derives it from `frequency_mhz`, on half-open bands: 6 below 2000 MHz, 7 from
2000 up to but not including 8000 MHz, 8 from 8000 up to but not including
20000 MHz, 9 at 20000 MHz and above. The resolution actually used is echoed in
the response.

Two per-cell quantities need their definitions read before use:

- `loss_db` is `metadata.peak_gain_db - gain(this cell)` — relative to the **peak
  over the cells actually evaluated**, the same rule `/api/v1/heatmap` applies. It
  is therefore never negative and is exactly `0` at the peak cell, and
  `total_path_loss_db` (`= free_space_path_loss_db + loss_db`) never falls below
  the free-space value. Note that the peak of the *grid* need not be the peak of
  the *beam*: a grid that does not contain the beam peak understates loss for
  every cell in it.
- `g_over_t_db` is `gain_db - 10*log10(temperature_k)` and is present only when the
  request supplies `temperature_k`. The temperature is a pure passthrough — the
  service models no antenna noise temperature of its own.

Like `/api/v1/gain/batch` and `/api/v1/heatmap`, this is a compute-heavy endpoint and
shares their admission-control limit and per-integration compute budget.
