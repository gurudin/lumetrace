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

Unsupported formats and known zero-byte files are completed in a single local
transaction. They retain truthful `unsupported`/`empty` extraction status, count
as ready in progress, and never enter remote-read or E5 queues. Reconciliation
also repairs their pending/failed jobs from older builds. Metadata/name search
remains available; no extracted text or vectors are fabricated.

Queue claiming, source reconciliation and vector commits reserve the SQLite
writer before reading revisions, avoiding deferred read-to-write upgrade errors
when another replica connection writes. Network reads and inference remain
outside write transactions. Previous transient lock failures are requeued with
a bounded retry count; actual extraction/model errors are not silently completed.

Verification: isolated metadata/late-response/deletion tests; complete Rust and
197 frontend tests/build passed. The opt-in real E5 test used existing model
assets and synthetic documents in temporary owner/member databases. Both devices
generated vectors, semantic recall contributed scores, restart retained results,
and version invalidation/deletion excluded stale content. No user documents or
live application database were read by that test.

Batch-skip regression: 467 unsupported files plus two synthetic text documents
immediately report 467/469 ready without a model or remote reads; two real E5
steps reach 469/469 for both isolated clients. The SQLite concurrency regression
holds a separate WAL writer and verifies job claiming waits rather than failing
to upgrade a stale read snapshot. Shared checks: 193 Rust tests passed (three
opt-in tests skipped), the real mixed-workload E5 test passed separately, and
197 frontend tests plus the production build passed.
