# Shared OpenAPI response descriptions

Prose blocks `include_str!`-ed into the `#[utoipa::path]` attributes in
`handlers.rs`. Each file here is shared by two or more endpoints (or is long
enough that inlining it as a string literal would drown the attribute); the
generated `openapi.yaml` repeats them per endpoint, exactly as the hand-written
spec did, but they are edited in one place.

Trailing newlines are stripped by YAML block-scalar folding, so files may end
with a newline freely.
