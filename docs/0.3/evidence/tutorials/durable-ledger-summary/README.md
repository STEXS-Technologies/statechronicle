# Durable tutorial qualification

Candidate `329fb7420070b883f29773f55c75d0c78c79d64a` adds the standalone
file-backed custom ledger tutorial. Both linked isolated qualification manifests
pass metadata, nine tests, exact CLI output, strict all-target Clippy and package
formatting on stable and Rust 1.85. The application was copied outside the repo
and only its umbrella dependency path was relocated. Lockfiles remained stable;
no registry packages were added relative to the root lockfile.

The summary manifest hashes unchanged library, root manifests, fuzz and workflow
files relative to `bbb1713`. Existing library qualification remains prior evidence,
not a new run. Tutorial Rust physical lines include tests/comments and do not
measure production-library complexity. The bounded file adapter is a concrete
consumer example, not backend-independent crash durability certification.

Preflight logs retain initial compile failures (wrong batch API and ambiguous
Read/Write method), the successful demonstration before warning cleanup, final
nine-test and strict Clippy runs, and AST graph update. These are distinguished
from the exact committed-source isolated qualification above. No original
protocol fixtures were edited. F0–F8 release gates remain incomplete.
