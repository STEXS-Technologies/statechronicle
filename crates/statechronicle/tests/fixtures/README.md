# Packaged reference fixtures

These are exact copies of the source files pinned in `provenance.json`. Test
code uses local package paths so archived crate tests do not require the parent
repository. Original `ci/` and documentation reference artifacts are unchanged.
The package checker verifies each copy against both the pinned Git object and
the current reference path before archiving. Never regenerate expected bytes
with the candidate implementation to make a failing test pass.
