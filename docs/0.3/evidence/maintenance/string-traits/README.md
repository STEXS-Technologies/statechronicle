# Private string trait owner

Final library/runner source: `d6aa9f417d765545301261ea9fe8df5aca256b05`.
Preceding control: `dea8a076d3e4e0e6c535a8dd1a2dbc7451688b49`.
Actual original release: `c418e98b91720188158c531cd5c2c4b8ab71cbcf`.

Nine string newtypes now invoke one private domain macro for Display, FromStr,
TryFrom<String>, TryFrom<&str>, Serialize and Deserialize: StateId, IntentId,
EventId, CommitId, SnapshotId, ProfileId, Operation, KeyId and Status. This
replaces 54 handwritten trait blocks with nine invocations and one template.
All 54 implementations still exist after compilation; this is a maintenance
owner, not a new runtime dispatch layer or a deletion of generated behavior.
No public macro, trait, crate, dependency, feature or wire schema is added.

Type-local constructors retain prefixes, limits, error variants and validation
order. Display and serialization still preserve the raw string without validating
it. That matters for eight public tuple fields, and for Status::from_static,
whose unchecked trusted-literal behavior remains unchanged. Deserialization
still obtains a String and calls the type's constructor, preserving serde error
conversion. Owned conversion still passes the owned String without another copy;
borrowed conversion and parsing still allocate one String. Status previously
forwarded two routes through try_from_str; that unchanged helper is exactly
Self::new(String::from(value)), now emitted directly by the shared shells.

[Owner audit](owner-audit.json) uses pinned tree-sitter to compare exact bytes
for nine struct declarations, their adjacent attributes, all selected inherent
methods and validators: 45 hashes match the preceding control. Field privacy,
derives, constructors and validator bodies stay unchanged. The macro itself is
not expanded by this audit; the external compiler/runtime corpus checks its
trait surface. Nonce retains its distinct base64 codec. Tenant/resource/subject
serde derives retain their original unchecked contract rather than being routed
through validation they did not previously perform.

## Behavioral qualification

[Final original/control/candidate campaign](20261116T143856Z-4178983/manifest.json)
compiles and executes one unchanged external consumer against all three actual
source archives. It compares 10,305 unique fully enumerated cases with **zero
differences**. Each of nine types has 1,128 string cases, ten malformed JSON
cases and seven malformed BCS cases. Strings cover every body length 0..260 for
ASCII, two-byte Unicode, emoji and combining marks, all 65 control characters
through U+009F, prefix collisions and selected non-control Unicode boundaries.

Each string row records constructor/parsing/owned/borrowed conversion, JSON and
BCS decoding, Debug/Display including formatter flags, raw JSON/BCS serialization
and protocol canonical bytes. Errors include Debug and Display. Invalid raw tuple
values are serialized intentionally: silently adding validation to that old
serialization API would change behavior. Status uses its existing trusted static
constructor with test-only leaked strings; this fixture is not an allocation or
latency benchmark. Malformed bytes include truncated/overlong lengths, invalid
UTF-8 and trailing bytes.

[Guarded direct driver](driver-d6aa9f4/manifest.json) records actual top-level
positive exit 0 and negative exit 1 with PYTHONOPTIMIZE=1. Its serialized child
runs require the complete unique case set. The negative archive replaces only
constructor-based deserialization with unchecked tuple construction. Both lanes
build and execute successfully; **5,608 rows differ**, and the checker refuses.
[The first isolated negative](20261116T144017Z-41759/manifest.json) independently
records the same result. Production is never mutated.

The earlier eight-type [9,160-case campaign](20261116T143605Z-4081186/manifest.json)
and [qualification](qualification-9f80012/manifest.json) are retained as successful
intermediate evidence. They do not qualify Status or replace the final reruns.

[Final immutable source qualification](qualification-d6aa9f4/manifest.json)
passes 897 finite workspace tests, 16 doctests, formatting, strict all-target/
all-feature Clippy and strict rustdoc, with archive and source-drift guards.
Fuzz targets compile through Clippy; unbounded fuzz tests are not executed.
[The frozen external consumer](../../compat/20261116T143925Z-4120/manifest.json)
passes legacy/additive tests, portable tests and bare-metal compilation.

## Source and compiled cost

[Final metrics](metrics-final.json), compared to the preceding library
[metrics](../index-value-declaration/metrics-a315713.json), remove 2,129 production
syntax tokens and 267 syntax lines. Whole-workspace totals are 118,924 tokens,
16,251 syntax lines and 1,490,324 raw tracked source bytes (13,062 fewer bytes).
Across the four changed domain modules, lib.rs and the new private owner, tokens
fall from 5,955 to 3,826, about 35.75%. This includes the template without
expanding its invocations; it measures repeated source maintenance, not generated
machine code. One private source module is added. The parser's branch count falls
1950→1941 because it does not expand the macro; no runtime branch reduction is
claimed. Its public-syntax count rises 1457→1458 for a crate-private macro reexport;
this is not a new exported API concept. Exact duplicate groups fall nine→four,
subject to the same macro-expansion limitation.

The final external consumer's control/candidate resolved locks and registry
feature sets match byte-for-byte/structurally under the same Rust 1.100.0 compiler.
Release binary size is 846,160→846,880 bytes (+720, about 0.0851%); .text is
494,054→494,070 (+16, about 0.00324%). Raw section reports and binary hashes are
retained in the final campaign. These measurements apply to this consumer and
compiler configuration, not every downstream application. The intermediate
eight-type consumer had unchanged .text; it is not substituted for final numbers.
Build logs retain actual durations, but cache/path differences make them unsuitable
for a controlled build-time claim. No latency or allocation improvement is claimed.
The small compiled delta is accepted for replacing nine repeated trait-maintenance
sites with one owner while preserving all validation and consumer behavior.

The four remaining exact groups are intentionally bounded shells: two error
conversions target distinct public error enums, and three player preparation
methods expose distinct Executor/Preparer capability surfaces while delegating to
the same Evaluation owner. Sharing them through a new public trait or cross-crate
generic mapper would add an API/ownership layer without removing preparation or
error classification behavior. Semantic duplication and whole-workspace feature
cost still require the broader roadmap audit; this is not full F6/F8 completion.
