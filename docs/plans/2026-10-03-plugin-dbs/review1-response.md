# Review 1 follow-up

The reviewer input in `review1.md` is unchanged. This follow-up fixes the
confirmed correctness and availability problems and removes duplicate work,
while retaining the existing backend and plugin contracts.

The later [review 2 follow-up](review2-response.md) corrects the failure policy:
every describe invocation error retries after the same backoff; only invalid
descriptor output shapes and decoding failures remain generation-cached.

## Changes

| Review item | Resolution |
|---|---|
| 1.1: repeated source names and aliases | Federation assigns each logical source an occurrence identity carried through lowering. SQL aliases and backend tags retain their original meaning. Sibling subqueries receive separate requests and tokens even when their source/binding names match. Ordinary sources default to no occurrence identity, preserving embedded explain output. |
| 1.2–1.3: snapshot latency and catalog identity | Query, explain, retry and schema prepare only requested names. Global naming/conflict discovery remains intact; list describes all exports concurrently. Descriptions survive catalog changes, while cached schemas are revalidated against new catalogs. Transient failures retry after a private five-second backoff; permanent describe/codec failures remain cached for the generation. Pointer identity only avoids repeated pure overlay validation. |
| 1.4: redundant bind negotiation | The ordinary plan is evaluated before offering a bind lookup. One exact accepted `IN :__keys` plan and token serve all groups of up to 64 outer rows. The indistinguishable second negotiation, batch fragment and call-order test knobs were removed. Rejected/non-exact bind offers retain an accepted ordinary scan; protocol errors still fail. |
| 1.5: arbitrary validation collection | Entity validation uses the actual virtual collection in its prepared overlay. Fake validation collections, static fallback and per-row catalog clones were removed. |
| 1.6: local buffering | The LocalSource documentation now states that mixed-query local leaves buffer their results and do not perform join-key index lookups. Existing local filter execution remains unchanged. |
| 1.7: alias and CLI schema | The local all-collection alias uses the core constant. CLI `api vdb schema <name>` exposes the existing schema command. |
| 2.1: obsolete execution path | Deleted the pre-engine SELECT planner/data source and renamed the registry-backed adapter without removing FederatedBackend or its write support. |
| 2.2: repeated overlay construction | Core `virtual_overlay` validates declarations, seeds all collection shells and applies each DDL batch once. Planning, SDK preparation and UI rendering share it; only the planner strips indexes. |
| 2.3: shared command types | List metadata and schema request records now live in `semantic_data::vdb`. App, UI and CLI share these types. UI field-by-field decoding was removed; absent optional fields retain their previous behavior. |
| 2.4: repeated normalization | The engine alone owns virtual join normalization. The app resolves candidate names without rewriting queries. |
| 3.1, 3.3: test overhead | Replaced timer-based blocked scans with a Notify gate and consolidated the duplicate job-store stub. Kept the independent stored database oracle; deleting it would replace the independent result check with the same federation implementation on both sides. Added repeated-alias cases to both focused core checks and the stored corpus. |
| 3.4: historical test cutoff | The already separate test-fixture correction now derives its legacy cutoff from the shared-ownership migration marker. Historical migration definitions and byte-equivalence assertions are unchanged. |
| 3.5: UI overlay rebuilds | A memo observes the local catalog, selected generation/revision and schema completion. Render synchronization compares memo completion identities instead of entire snapshots. Unrelated renders reuse the same catalog and issue no extra RPCs. |
| 3.6: repeated catalog access | Routing reuses its catalog for write checks. Local text and parameters still delegate unchanged; forwarding parsed ASTs would weaken compatibility with text-only/backend-specific adapters. |

## Deliberately retained

- Projection remains an optional protocol hint. Omitting it preserves complete
  entities and computed dependencies, as established by spike S3.
- Computed-attribute error behavior remains consistent with embedded queries.
  Structured DbError changes, revision API redesign and replacing
  SourceCapabilities would broaden this corrective review.
- The visitors in 2.5–2.6 remain unchanged. A general traversal framework is
  unnecessary for these fixes; the existing expression visitor mutates owned
  queries, whereas collection discovery borrows them without cloning.
- The checks in 2.7 remain at public SDK adapter/source boundaries because
  direct callers can supply malformed plans; the pure validation cost is small.
- The fast engine corpus and cheap SQL preflight remain alongside the stored
  oracle. The stored suite reuses two initialized apps; splitting it into many
  fresh scopes would increase initialization cost and duplicate scaffolding.
- The small CLI validation and forwarding tests in 3.2 remain. Schema parsing
  replaces the previous list parsing case rather than adding another test.

## Validation

Passed: 470 core tests with all features, the indexed-plan benchmark
regression, 33 SDK tests with testing enabled, five VDB data tests, 105 UI-core
unit tests and 22 integration tests, four virtual-collection UI tests, four
wrapper tests, four package-registration tests and two CLI tests including the
schema parsing case.

The stored app oracle/lifecycle test passed all 22 queries across four plugin
modes (88 comparisons), four direct plain-alias assertions, native SDK selected
binding with one actual scan, and runtime-schema/lifecycle checks. Its initial
three-test invocation passed two tests and failed the cheap smoke test because
that test assumed a particular corpus query was last. The smoke test now checks
its qualified join predicate explicitly and passed its targeted rerun. The
final workspace short-format check, formatting and whitespace check passed.
All check/test commands used the Nix devshell and the main clone's shared
target directory. No broad full-workspace test rerun or new commit was made.
