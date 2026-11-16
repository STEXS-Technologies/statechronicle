# Index value-declaration owner

Production source: `a3157135199f7c86f74f8eb79cb7bad7b483d11c`.
Preceding control: `c9a4605e460531d1d2b56a5b1adb4fe93ddfe55b`.
Original release: `c418e98b91720188158c531cd5c2c4b8ab71cbcf`.

The index's two declaration checks now import the existing public
`profiles::atomicity::declares_value_leg`. The deleted private implementation
had the same three ordered `contains_key` calls and short-circuit OR expression.
Neither implementation parses values, allocates, performs I/O, nor returns an
error. A partial declaration must route into subsequent validation rather than
being mistaken for an absent value leg. Call positions and later tuple parsing,
matching, and error precedence remain unchanged. No new public API, dependency,
crate, feature, or wire format is introduced.

[Contract evidence](contract-a315713/manifest.json) retains an identical test
inserted into isolated actual-original and preceding-control archives, and run
unchanged on the candidate. It checks all 512 combinations of absence, null,
empty string, nonempty string, number, boolean, array, and object across the
three keys, with an unrelated input present. Each result is compared to the
presence-only contract for both the index name and profile owner. Full index
suites passed: original 17 tests; control and candidate 25 each. These counts
include the added contract test. The original lock is retained unchanged and
is different from the candidate lock; this is behavioral compatibility evidence,
not matched performance evidence.

An isolated negative control changes the profile owner's OR operators to AND.
It compiles and fails the contract at runtime (exit 101), demonstrating detection
of the partial-declaration regression through the imported index name. The
mutation and compressed raw logs are retained; production is not mutated.

[Immutable source qualification](qualification-a315713/manifest.json) records
897 finite workspace tests, 16 doctests, formatting, strict all-target/all-feature
Clippy, and strict rustdoc passing, with archive and source-drift guards. Fuzz
binaries are compiled by Clippy; unbounded fuzz tests are not executed.

[Syntax metrics](metrics-a315713.json) report 121053 production tokens, 1950
parsed branches, 16518 production syntax lines, and 1457 public syntax items.
Against the previous library checkpoint this removes 37 tokens and four syntax
lines, with unchanged branch/public counts. The metric's 1503386 raw tracked
bytes include colocated test text; that raw count increases by 1525 bytes even
though production syntax shrinks. No macro expansion or compiled size inference
is made. This small deletion does not establish the broader hotspot budget.

Index value-pair recovery remains separate from profile atomicity validation:
the index attributes already committed batch-local pairs, while the profile
validates complete transfer groups and net-zero constraints with different
refusal semantics. Sharing the declaration predicate does not merge those roles.

No allocation, binary-size, or latency improvement is claimed. The imported
predicate has exactly the old expression and introduces no dispatch layer, but
compiler inlining and whole-workload costs still require the final matched
performance campaign. This is a bounded F6 ownership cleanup, not a full F6/F8
qualification.

The [frozen external consumer](../../compat/20261116T141657Z-3511401/manifest.json) passed legacy/additive tests, portable resolution/tests, and bare-metal compilation against the candidate. The [checkpoint verifier](verify-checkpoint.py) checks statuses, counts, log hashes, and exact equality of the original/control predicate with the retained profile owner.
