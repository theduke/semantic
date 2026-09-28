# Durable registration certificates

2026-09-28. This implementation supersedes the withdrawn package-local proof in
[plan.md](plan.md) and [review.md](review.md). The follow-up explicitly authorizes
durable metadata and a forward migration. Historical migrations are unchanged.

## Invariant and fast path

A certificate means that the existing package validator succeeds and replaying
every stored migration's DDL, then updating package metadata, produces exactly
the same complete catalog storage snapshot. This is checked independently on the
live catalog and on its reconstructed persisted representation. The loader can
canonicalize relationships, so each representation is checked against its own
snapshot and both SHA-256 fingerprints are recorded. This is an observed fixed
point of the actual algorithm, not a second implementation of DDL semantics.

The certificate includes an algorithm version and exact normalized package
digests, including metadata and all migration contents. Certificates are generated
while encoding catalog writes, carried in the catalog metadata row, and committed
atomically with those writes. They are installed in memory only after storage
commit and catalog compare-and-swap succeed. Every runtime catalog commit uses
this encoder, including DDL, package updates/repairs, and internal collection
changes. Open-time core changes use it too. Proof generation never executes data
operations and unsuccessful certification does not introduce a new validation
error into a catalog write.

On reopen, the existing catalog scan reads the metadata. After core startup
reconciliation, the installed catalog fingerprint is checked in memory without
a second storage scan or migration replay. This also rejects stale proof after
implicit reconstruction of a missing built-in collection. In memory,
certificates are bound to the exact `Arc<Catalog>`
and catalog version. Public snapshot replacement and both SharedCatalog CAS APIs
therefore invalidate them automatically, even when the same Arc is reinstalled.
Unrelated data writes retain certificates; unrelated catalog writes recompute
them and retain proof for each package whose replay remains a fixed point.

Registration normalizes/hashes only its input package, checks the bound proof,
and retains the storage revision fence. A catalog identity/version fence also
detects concurrent external catalog replacement. Both fences use normal retries.
The fast return performs no catalog snapshot construction, global validation,
catalog clone, migration DDL replay, storage entity read/scan/write, commit,
revision increment, catalog version increment, or change-feed event. Completion
reports `registration_path="unchanged_certified"`. Ordinary full registration and
its diagnostic phases remain the fallback.

## Why the earlier blockers remain correct

- Unrelated stale projections invalidate identity on replacement; certification
  also refuses a catalog whenever full replay repairs any unrelated projection.
- `Schema -> Untyped` history that advances `next_field_id` on every replay never
  receives a certificate. Every registration preserves the existing mutation.
- Loading an unrelated self-inheriting class is not treated as validation.
  Certification runs full validation/replay and refuses the proof; registration
  continues reporting the original error, including after reopening.
- A `Log`-policy mismatch never receives a certificate, even after changed package
  metadata is persisted. Each registration still logs and replays stored DDL.

## Upgrade and compatibility

The actual base-then-filestore startup sequence exposed a second cause of
repeated work: both packages historically upserted `semantic:parent`,
`semantic:title`, and `semantic:description` with different module ownership.
Parent also alternated the legacy unrestricted `Ref("id")` spelling and intrinsic
`Ref(None)`. Full registration changed those definitions on every alternating
call, causing real catalog writes and collection scans even with zero migrations.
Certifying that sequence without fixing the oscillation would be incorrect.

Both packages now append `010_shared_attribute_ownership`. Its identical frozen
definitions live in their `shared` module, and current package declarations expose
these three attributes in `package.modules["shared"]` instead of the root module.
Canonical attribute IDs, class references, and the parent's Restrict deletion
behavior are preserved. Each package remains independently installable. The
first nine migrations of both packages have exact serialized SHA-256 regression
snapshots captured before this change; no historical constructor is changed.
Tests cover upgrading old nine-migration packages and repeated complete startup
in both installation orders, before and after reopening.

Forward core migration `009_registration_proofs` adds the bookkeeping attribute.
Its ordinary atomic catalog rewrite certifies healthy legacy packages. Repairs
that are still necessary are left to normal registration. There are no separate
certificate-only writes or fabricated change events/revisions.

Missing, malformed, obsolete, or fingerprint-mismatched certificates always fall
back. An older binary that rewrites the catalog drops this optional metadata;
registration stays conservative until a subsequent catalog mutation regenerates
it. Reopening a certificate-less database does not silently perform a new audit
or write. Changing normalization, loading, validation, or reconciliation semantics
requires advancing the certificate version and a forward core migration so
existing databases are certified for the new algorithm. Certificates are trusted
internal bookkeeping, not authentication of files edited outside the storage API.

## Cost and measurements

The explicit tradeoff is additional schema-write work: certification validates
and replays each installed package on both catalog representations. That work can
scale with catalog size and package count. Ordinary data writes and certified
registration do not pay it. Open adds an in-memory snapshot hash to its existing
catalog load; open still scales with catalog size. This change does not optimize
PostgreSQL's surrounding load/save adapter.

Release Criterion measurements on this development machine, 10 samples with
0.1-second warmup and 0.2-second measurement targets, including package cloning
and normalization, excluding setup/open. Both updated default packages are
installed in every fixture:

| Package / fixture | Warm memory estimate | First registration after redb reopen | Mean redb open |
| --- | ---: | ---: | ---: |
| base, empty | 773 µs | 830 µs | 21.4 ms |
| filestore, empty | 787 µs | 843 µs | 21.4 ms |
| base, 1,000 rows | 776 µs | 829 µs | 21.4 ms |
| base, 100,000 rows | 795 µs | 815 µs | 21.5 ms |
| base, 100 unrelated attributes | 772 µs | 824 µs | 26.1 ms |
| base, 1,000 unrelated attributes | 781 µs | 881 µs | 75.2 ms |
| base, 1,000 unrelated classes | 773 µs | 858 µs | 62.1 ms |

These are short local estimates, not p95 guarantees or measurements on the
reported slow database. Entity count and unrelated schema are varied separately.
The deterministic CI guard asserts zero full-catalog snapshots, historical DDL
replays, collection/index scans, row reads, writes, commits, and change events
after reopening and an unrelated DDL addition. Tests also cover data writes,
external replacement during the fast fence, retry exhaustion, failed commits,
invalid-row repair rejection, stale/obsolete metadata, legacy upgrade, and
non-reexecution of historical data operations when a new migration is added.
Eight generated collection-kind histories compare public registration against
full replay before and after reopening, covering both accepted certificates and
histories that must keep reconciling. The base reopen test includes filestore as
another installed default package.

Validation uses the Nix development shell: data/core/base/redb tests, workspace
`cargo check --quiet --message-format=short --workspace --all-targets`, `cargo fmt`,
the package-registration release benchmarks, and `git diff --check`.
