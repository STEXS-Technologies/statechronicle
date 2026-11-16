# Consumer/migration documentation check

Candidate `b5d00d747e9ba834648399f21bff0547ba1f467a` rewrites the root entry
journey and adds consumer and migration maps plus operator recovery decisions.
The manifest retains changed-file hashes, local Markdown target checks and
stable/Rust 1.85 import-resolution logs. The documented public imports were
compiled in a temporary external application using the existing tutorial
manifest/lockfile, replacing only its path dependency and main source. No new
registry resolution or lockfile edits were accepted.

`check.py` reproduces the local checks at the pinned candidate checkout. It
requires populated Cargo caches and installed toolchains. It checks link target
existence, not network availability or rendered layout. Imports resolve but this
is not a new runtime test. Library Rust, manifests, lockfile, tutorials, fuzz and
workflows are unchanged; their prior qualification is not relabelled as a new
run. This checkpoint does not complete the release gates.
