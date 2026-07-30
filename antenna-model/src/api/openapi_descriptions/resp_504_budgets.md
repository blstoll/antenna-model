A server-side wall-clock budget was exceeded. Two distinct budgets map here, told
apart by the machine `error` code:

- `request_timeout` — the whole request exceeded `server.request_timeout_secs`
  (default 30 s, roadmap S2). The server stops waiting and responds, but the
  background compute is NOT cancelled and runs to completion.
- `computation_budget_exceeded` — a SINGLE aperture integration exceeded
  `performance.integration_budget_ms` (default 30 000 ms, roadmap S3), aborting
  that integration cooperatively. This caps one integral, not the whole request;
  a large fan-out (heatmap/batch) can still spend `budget × points` of background
  CPU after an S2 504.

504 (a 5xx) is used rather than 408 because the overrun is server-side and
deterministic in the request payload — there is no honest Retry-After; the remedy
is a smaller request, not waiting.
