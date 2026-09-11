# Isolated local semantic indexes

`workspace_search::Workspace` exposes the existing extraction, multilingual E5,
SQLite chunks/embeddings, persistent ANN and lexical search implementation to
caller-owned workspace replicas. It does not start a watcher, register a space,
switch the local database or perform remote I/O. Model installation is device
scoped; its loaded model/lifecycle is shared within the process, while each
workspace owns a separate database and ANN snapshot.

The caller must authorize every operation, reconcile a complete visible source
set with monotonic revisions, download revision-checked input into a private
temporary file, and revalidate before supplying it. `prepare` immediately drops
superseded chunks and absent documents; late extraction is rejected by revision.
Call `step` incrementally outside UI, transport and catalogue locks. Search
combines lexical matches with dense recall from the same local E5 engine, without
requiring a chat provider or restricting dense recall to literal keyword hits.

Verification: isolated metadata/late-response/deletion tests; complete Rust and
197 frontend tests/build passed. The opt-in real E5 test used existing model
assets and synthetic documents in temporary owner/member databases. Both devices
generated vectors, semantic recall contributed scores, restart retained results,
and version invalidation/deletion excluded stale content. No user documents or
live application database were read by that test.
