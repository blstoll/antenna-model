Request body exceeds the configured maximum size
(`server.max_body_size_bytes`, default 10 MB).

Enforced on both framings (roadmap S1b): a declared `content-length` over the
limit is rejected before the body is read, and a body sent with
`Transfer-Encoding: chunked` is rejected as soon as the bytes read exceed the
limit. The chunked path cannot report a byte count in the message, since the
size is never declared.
