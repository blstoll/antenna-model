Admission control (roadmap S4): the server is already running the maximum
number of concurrent heavy requests (batch / heatmap / h3-heatmap), configured
by `performance.max_concurrent_heavy_requests` and shared across those three
endpoints. The request was rejected immediately (never queued). Unlike the 504
timeout, this is a transient overload — a slot frees as soon as an in-flight
heavy request finishes — so a `Retry-After` header is included. Never returned
when the limit is 0 (the default; admission control disabled).
