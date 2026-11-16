# Self-contained archive test qualification

At pre-fix `4c042e4`, Cargo successfully made thirteen archives, but testing the
unpacked umbrella crate failed to read `../../../ci/wire-0.2/history.json`.
Source inspection found four test files reaching five references outside their
package. The initial packaging/test logs, commands and failing umbrella archive
are retained here. Package generation alone was not a closure test.

`907c359` changes test paths to local, byte-identical reference copies with
pinned Git/path/SHA-256 provenance and adds `scripts/check_packages.py`. Original
wire/consumer fixtures and historical producer evidence remain unchanged.
Production Rust/manifests/lock and root lint policy also remain byte identical.
Copies add approximately 389 KB of test data/source, not production code.

The checker creates actual archives outside the repository, unpacks all thirteen,
patches internal dependencies solely to those archives and verifies every resolved
internal source location. Registry packages may not grow beyond the committed
root lock. The first checker run passes all 884 finite tests and 16 doctests but
fails strict Clippy: its synthetic workspace omitted the existing root lint
configuration, changing the default error-size threshold from 256 to 128.
That failed manifest and its archives/logs remain unqualified for the full check.

`e6e0dec` copies the exact committed Clippy policy into the synthetic workspace;
no threshold, lint suppression or library type changes. Its full fresh archive
sequence passes metadata/package/isolated resolution, 884 finite tests,
16 doctests and strict Clippy. All thirteen archives, normalized isolated lock,
commands, compiler/source/config hashes and logs are retained by its linked
manifest. The external candidate consumer/wire/portable/bare-metal matrix passes
at 907c359; only the checker changes between that source and e6e0dec.

The checks use the unchanged pre-release version 0.2.0 with local archive patches.
They do not publish, contact a registry service for qualification, or assert that
new APIs are present in old registry packages. Final version changes require
rerunning this gate. This remains partial F8 evidence; other roadmap requirements
are not declared complete.
