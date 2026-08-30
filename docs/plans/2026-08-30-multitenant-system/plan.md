# Multi-Tenant Server, Identity, and Authorization Implementation Plan

Date: 2026-08-30
Status: proposed
Scope: `semantic_server`, `semantic_app`, `semantic_rpc`, a dedicated root control database, and new `semantic_control_plane` and `semantic_authz` crates

## 1. Executive decision

Evolve the server into an explicitly multi-tenant control-plane/data-plane architecture:

- A **physically separate root control database** is the source of truth for global accounts, global credentials/sessions/tokens, tenants, auth scopes, the database registry, cross-database grants, root authorization policy, and platform security audit events. It is never exposed through the generic database RPC API.
- Each user-facing database remains a separate database opened through the existing `DbProvider` abstraction and can own a second, database-local identity and authorization domain: local accounts, credentials, database-bound tokens, groups, global-principal bindings/shadow subjects, local policy, and local audit records. Local identity data is administered through that database and has no authority outside it.
- Every registered database has both an immutable UUID `DatabaseId` and a globally unique `DatabaseName`. Login, connection, query/session selection, and administrative APIs may accept either as an untrusted `DatabaseSelector`; the server resolves it immediately and all authoritative state thereafter uses the UUID.
- Authentication establishes an immutable, domain-qualified `AuthenticatedPrincipal`: either a root/global principal or a principal local to exactly one database UUID. A request may propose an auth scope and database, but the server validates both against authoritative state before opening or using any database. A client-supplied scope, UUID, name, query parameter, RPC payload field, token routing prefix, or WebSocket selection is always an **untrusted selector**, never proof of access.
- A root **auth scope** is a named security context that can grant access to one or more registered databases. Global accounts and tokens may be allowed into multiple auth scopes; a connection has at most one active auth scope and one active database at a time. Local principals do not enter root auth scopes. This keeps “who are you?”, “which security context are you using?”, and “which database are you addressing?” separate.
- For a global principal, entry and operations normally require two safe authorization gates: the root domain must permit access to the selected database **and** that database's local domain must permit the operation for its bound/shadow subject. The local policy cannot widen a root ceiling. A database-local principal is cryptographically bound to one UUID and is evaluated only by that database's local policy.
- Platform administrators receive explicit, auditable platform capabilities. Operational administration is separate from tenant-data access; data access uses an audited, database-bound elevation, with a distinct time-limited break-glass capability if bypass of local policy is deliberately enabled. Administrators never impersonate the internal `system` principal and do not gain generic query access to the root control database.
- Implement the authorization model from the beginning behind a dedicated, storage- and transport-independent `semantic_authz` crate. Initially it decides coarse database actions. Later phases add relation tuples, groups/usersets, object hierarchies, typed dynamic conditions, and object-level query enforcement without replacing the core API.
- Use opaque, revocable bearer tokens initially rather than self-contained JWTs. Authorization depends on mutable membership and policy; global tokens are verified in the root database and local tokens in their owning database. Authoritative lookup gives immediate disable/revocation semantics and avoids stale privileges embedded in long-lived claims. External OIDC/JWT validation can be added behind the same authenticator interface later.
- Keep the existing auth-less, single-user flow as an explicit deployment mode with exactly one configured default `main` database, no public identity assertions, and no required root database. It binds to loopback by default and cannot be mixed with multi-tenant authentication/selection. Multi-tenant mode fails closed if authentication, the root database, migrations, or bootstrap state are unavailable.

The central request invariant is:

```text
credential
   -> domain-qualified authenticated principal + token/session constraints
   -> canonical DatabaseSelector -> DatabaseId UUID
   -> validated active auth scope/root boundary (global principals)
   -> database-local subject/binding and policy
   -> leased database handle
   -> command/object authorization
   -> operation
```

Auth-less mode intentionally has a smaller, separate invariant:

```text
explicit single_user_authless mode
   -> fixed configured main database
   -> internal LocalSingleUser principal (not system/global/local-network identity)
   -> leased database handle
   -> operation
```

No alternate HTTP, WebSocket, file, RPC, admin, or background-job path may skip this chain.

OWASP's current multi-tenant guidance recommends establishing tenant context early, binding it to authenticated state, never trusting a client-supplied tenant identifier without validation, isolating session/cache keys, and enforcing tenant ownership in the data-access layer ([OWASP Multi-Tenant Application Security](https://cheatsheetseries.owasp.org/cheatsheets/Multi_Tenant_Security_Cheat_Sheet.html)). This plan follows those principles while using separate databases as the primary tenant isolation boundary.

## 2. Terms and identity boundaries

Use distinct strongly typed IDs; do not continue overloading `DbScopeId` for unrelated concepts.

| Term | Meaning |
|---|---|
| `GlobalPrincipalId` | Stable identity of a root account or service account that may span databases |
| `LocalPrincipalId` | Stable identity meaningful only together with its owning `DatabaseId` |
| `PrincipalRef` | Domain-qualified principal: `Global(id)`, `DatabaseLocal { database_id, id }`, or a non-network internal kind |
| `TenantId` | Administrative/customer boundary; owns global memberships, auth scopes, and normally databases |
| `AuthScopeId` | Root/global active security context; combines membership/policy and can grant several databases |
| `DatabaseId` | Immutable UUID identity of a database; canonical key in all security and persistence state, never a URI or path |
| `DatabaseName` | Globally unique, normalized, human-usable database name; mutable only under the explicit rename policy |
| `DatabaseSelector` | Untrusted input containing either a UUID or a name; it must be canonicalized to `DatabaseId` before authz/open |
| `AuthSessionId` | Persisted login session qualified by its owning root or database-local identity store |
| `ConnectionSessionId` | Ephemeral HTTP/WebSocket client state such as current auth scope/database |
| `TokenId` | Public lookup/revocation identifier for an opaque token; not the token secret |
| `ObjectRef` | Authorization target `{ domain, database_id?, namespace, type, id }` |
| `Verb` | Namespaced operation such as `database.query`, `entity.read`, or `policy.manage` |

`TenantId` and `AuthScopeId` are intentionally not synonyms. A tenant can define several scopes (`editors`, `billing`, `automation`), and a controlled platform scope may span databases. Default policy must reject cross-tenant scope-to-database grants; only an explicit platform-level action may create one. Local principals never participate in a cross-database auth scope.

Identity domains are never inferred from usernames or email addresses. `global:user/123` and `database:<uuid>:user/123` are different subjects even if their profiles match. A database can assign local permissions to a global user only through a root-validated external-principal binding (usually represented locally by a credential-less shadow subject). Local administrators cannot mint global subject references.

Rename the current user-selected database concept from `DbScopeId` to `DatabaseSelector`/`DatabaseId` over a compatibility period. Reserve “scope” for authorization context. Until the rename completes, serialization may continue using `scope_id`, but internal APIs must parse it as a selector and canonicalize to the UUID so a name, scope, and database identity cannot accidentally share a key.

## 3. Current-state assessment

The repository already contains several good seams:

- `crates/app/src/db.rs` defines the backend-neutral `SemanticDb` and `DbProvider` abstractions.
- `crates/app/src/scope.rs` can open provider URIs, retain descriptors, resolve a request/session/default selection, share system-visible scopes, and retire idle handles.
- `crates/app/src/context.rs` centralizes database resolution for app commands.
- `crates/server/src/router.rs` resolves a scope from `X-Semantic-Scope` or `?scope=` for HTTP, while `crates/server/src/ws.rs` creates one `AppSession` per socket and retains its selected scope.
- `crates/server/src/auth.rs` has a `PrincipalResolver` seam and clearly labels header identity as test scaffolding.
- `crates/data/src/bundles/auth/mod.rs` contains an unused initial user class and `_semantic.auth` collection that can seed the control-plane schema work.
- The file service resolves its database through `AppRequestContext`, so it can share the same enforcement path once file authorization is added.

The existing implementation is scaffolding, not a secure multi-tenant system:

| Current behavior | Problem | Required change |
|---|---|---|
| `NoAuthPrincipalResolver` is the server default and returns `Principal::system()` | Every unauthenticated request has the most privileged internal identity | Replace it with explicit mode startup: authenticated domain principals in multi-tenant, or a private fixed-main capability in auth-less mode; never map a request to `system` |
| `HeaderPrincipalResolver` trusts an arbitrary principal header | Identity spoofing if used outside tests | Keep behind test/dev feature or remove from public production configuration |
| `Principal` contains only ID and kind | No identity domain/database binding, authentication method, tenant memberships, token/session ID, auth time, or security version | Introduce domain-qualified immutable `AuthenticatedPrincipal`/`AuthenticationContext` |
| `ScopeManager` state is only in memory | Registered databases and grants disappear on restart; instances disagree | Persist descriptors and access mappings in the root DB; cache only opened handles |
| Scope entries are keyed by `(principal, scope_id)` | The same physical DB can be opened repeatedly; a principal is treated as ownership proof | Key handles by `DatabaseId`; authorize separately before handle lookup |
| `semantic.scope.open` accepts an arbitrary URI and permits `system` visibility | Path traversal, SSRF/provider abuse, privilege escalation, and control-plane bypass | Restrict raw URI registration/opening to platform DB-admin APIs; normal users select registered IDs |
| HTTP scope headers/query parameters and RPC `scope_id` fields are accepted as selectors | ID manipulation can cross boundaries once multiple users exist | Authenticate first, canonicalize UUID/name to UUID, then authorize every resolved selector; return a non-enumerating denial |
| HTTP requests have no persistent app session; WebSocket sessions are process-local counters | No revocable login/session model, unsafe restart behavior, ambiguous session terminology | Add persisted `AuthSession`; keep connection state separate and random-ID based |
| The WebSocket principal is fixed at upgrade and every message is spawned concurrently | Token revocation is not observed; `scope.use` races with concurrent commands | Bind an auth snapshot/version, add ordered connection-state transitions, and revalidate sensitive requests |
| RPC clients have no token/cookie configuration | Native and browser clients cannot authenticate consistently | Add credential providers and cookie-aware/browser-safe modes |
| `semantic.db.query` accepts arbitrary text queries | Future object authorization can be bypassed through scans, joins, aggregates, and side channels | Gate it at DB-wide permission until authorization-aware planning exists |
| File download uses a selected database and bare file ID | IDOR and cross-tenant blob disclosure if selection is not authorized; browser media cannot attach a bearer header easily | Apply the same context check and add authenticated cookies or short-lived signed file capabilities |
| App errors lack permission/token/session distinctions | Clients cannot handle expiry vs forbidden vs invalid selection cleanly | Add stable authn/authz error codes with safe messages |
| Default DB and blob paths are hard-coded around `default` | No root/data separation or tenant lifecycle | Add root path plus database/blob registry descriptors |
| There is no database-local identity store or global-to-local mapping | A root-only account system cannot provide database sovereignty; duplicated usernames risk identity confusion | Install a protected local auth package in each database and use domain-qualified principals plus root-attested bindings |
| There is no permanent public database name model | Human selectors could drift into security state or collide | Add globally unique normalized names, canonical UUID resolution, and explicit rename/reuse rules |

The existing `semantic_data::bundles::auth` schema has only username and primary email, no package constructor, indexes, credential/token/session records, grants, or runtime integration. Reuse its lessons, but split the final storage contracts deliberately: global identity belongs in `semantic.control` in the root database, while a versioned `semantic.local_auth` package is installed in every multi-tenant user database. Do not maintain two accidentally divergent schemas behind one unqualified account type.

## 4. Target crate and dependency architecture

Add two crates and preserve the current layering:

```text
semantic_data        semantic_db_core/backends
       \                    /
        v                  v
            semantic_authz
                 |
                 v
       semantic_control_plane
                 |
                 v
           semantic_app
                 |
                 v
       semantic_rpc <- semantic_server <- semantic_ui
```

### 4.1 `crates/authz` (`semantic_authz`)

This is the required standalone permission-core crate. It must not depend on axum, RPC transports, `semantic_app`, a database backend, or global mutable state. A dependency on neutral `semantic_data` values is acceptable.

Core modules:

- `ids.rs`: typed identity-domain, subject, object, relation, verb, tenant, scope, and database identifiers;
- `model.rs`: versioned authorization model, object types, relations, permission rewrite expressions, schemas, validation;
- `tuple.rs`: relation tuples and subject/userset references;
- `condition.rs`: typed, deterministic condition AST and validated evaluation context;
- `request.rs`: `CheckRequest`, `BatchCheckRequest`, consistency/revision requirements;
- `decision.rs`: allow/deny, stable reason codes, dependency revision, optional bounded explanation;
- `evaluator.rs`: cycle-safe/budgeted graph and condition evaluator;
- `store.rs`: async read traits for models, tuples, attributes, and revisions;
- `cache.rs`: cache interface/key types, not a concrete global cache;
- `error.rs`: malformed model, indeterminate, budget exhausted, stale snapshot, and storage errors;
- `testing.rs`: in-memory store/model builders behind a feature.

Public entry points should resemble:

```rust
pub struct CheckRequest {
    pub subject: SubjectRef,
    pub verb: Verb,
    pub object: ObjectRef,
    pub domain: AuthorizationDomain,
    pub auth_scope: Option<AuthScopeId>,
    pub context: EvaluationContext,
    pub consistency: ConsistencyRequirement,
}

pub trait AuthorizationEngine: Send + Sync {
    async fn check(
        &self,
        request: CheckRequest,
    ) -> std::result::Result<Decision, AuthorizationError>;

    async fn check_many(
        &self,
        requests: Vec<CheckRequest>,
    ) -> std::result::Result<Vec<Decision>, AuthorizationError>;
}
```

`AuthorizationDomain` is `Root` or `Database(DatabaseId)`. The evaluator is shared, but its stores, model revisions, tuple namespaces, cache keys, and trusted attribute providers are domain-separated. The app composes root and database decisions; the core evaluator must not silently union results from two domains.

No `Result<T>` aliases should be introduced; follow the repository convention of explicit `std::result::Result<T, E>`/`Result<T, E>` signatures.

### 4.2 `crates/control_plane` (`semantic_control_plane`)

This crate owns root-domain models and services:

- global account, service-account, credential, session, and token lifecycle;
- tenant, membership, auth-scope, database descriptor, and grant lifecycle;
- root schema package and ordered migrations;
- `ControlPlaneStore` typed persistence trait and a `SemanticDbControlPlaneStore` adapter;
- password hashing/token secret verification and secret-redacting types;
- authentication and scope resolution services;
- a storage adapter implementing `semantic_authz` store traits;
- audit event creation and an asynchronous/bounded audit sink;
- bootstrap and recovery operations.

It also owns the root half of global-to-local binding workflows and signed/internal binding attestations. Database-local repositories and services live behind neutral traits used by `semantic_app` (or a small `local_auth` module/crate if implementation pressure warrants it); they must not make the root database depend on arbitrary user databases.

It may depend on `semantic_db_core`, but it must not expose its root `Db` as a generic user database. All callers use typed repository/service methods. Keep provider opening in `semantic_app`, preventing a dependency cycle.

### 4.3 `semantic_app`

The app layer coordinates authenticated requests, persistent registry data, authorized DB leases, commands, and object-store access. It owns enforcement points and passes pure checks to `semantic_authz`; it does not parse HTTP credentials.

### 4.4 `semantic_server`

The server parses Authorization/cookie/upgrade data, establishes request IDs and network context, creates `AppRequestContext`, maps errors to HTTP/WebSocket protocol semantics, and exposes login/admin routes. It does not make permission decisions itself.

### 4.5 `semantic_rpc` and UI

RPC defines selection/session metadata and credential-capable clients. The UI adds login/logout, auth-scope/database selection, and account/token management surfaces after server APIs stabilize.

## 5. Root control database

### 5.1 Isolation and startup contract

- Configure a dedicated `SEMANTIC_ROOT_DB_URI` (default local development path: `<data-dir>/db/root-control`). It must not equal or resolve to any registered tenant database URI.
- In `multi_tenant` mode, open it before the public listener. Apply control-plane migrations, verify schema/version, load bootstrap state, and only then mark readiness. `single_user_authless` does not open or depend on it.
- Do not register its URI with `ScopeManager`, return it from scope listing, pass it to `DbProvider::open` for an external principal, or expose root collections through `semantic.db.*`.
- Restrict filesystem ownership/permissions for local redb. For remote backends, use a separate least-privilege database credential and secret manager reference.
- Back up and restore the root DB separately from tenant data; document their consistency point and disaster-recovery order.
- Start multi-tenant mode in read-only/unready state if migrations are pending or the root DB is unavailable. Do not silently fall back to no-auth.

### 5.2 Root schema

Create a versioned `semantic.control` package with strict collections and unique indexes. Suggested records:

| Collection | Key fields and purpose |
|---|---|
| `global_accounts` | `global_account_id`, status, display name, normalized login identity references, security version, timestamps |
| `global_login_identities` | identity ID, global account ID, kind (`username`, `email`, external issuer/subject), normalized value, verified state; unique `(kind, normalized_value, issuer)` |
| `global_password_credentials` | global account ID, PHC Argon2id string, algorithm/rehash metadata, changed/compromised timestamps |
| `global_service_accounts` | global principal ID, owner tenant, status, display metadata |
| `tenants` | tenant ID, slug, status, lifecycle timestamps, quotas/tier metadata |
| `tenant_memberships` | tenant/account, status, base relationship; unique pair |
| `auth_scopes` | scope ID, tenant/platform owner, name, status, root model ID/revision, default database UUID |
| `auth_scope_memberships` | scope and subject/userset relation; supports account, service, and group membership |
| `databases` | UUID database ID, globally unique normalized name plus display spelling, owner tenant, provider scheme, encrypted/secret-referenced descriptor, status, schema version, blob-store binding, created/deleted timestamps |
| `database_name_history` | database UUID, prior normalized/display name, changed-by/time, alias expiry/reservation status; never used as an authority-bearing identifier |
| `auth_scope_databases` | scope, database ID, coarse relation (`viewer`, `editor`, `owner`), constraints; unique pair/relation |
| `global_tokens` | token ID, global principal, type, verifier digest and pepper key ID, allowed/default scopes, UUID database audience ceiling, status, issued/expiry/last-used/revoked timestamps |
| `global_auth_sessions` | session ID, global principal, verifier digest, CSRF metadata, auth method/level/time, expiry/idle deadline, active/default scope/database UUID, status |
| `external_principal_bindings` | database UUID, global principal ID, desired local shadow subject ID/status, provisioning generation/state/error, root grant and timestamps; unique `(database_id, global_principal_id)` |
| `global_groups` and `global_group_memberships` | root-scope nested subject sets with cycle validation |
| `root_authz_models` | immutable versioned root model text/AST, hash, status, created-by/time |
| `root_relation_tuples` | root scope/model, object, relation, qualified global subject/userset, optional condition, created/revoked metadata |
| `root_policy_bindings` | typed root dynamic policy/condition bindings where tuples alone are insufficient |
| `root_authz_revisions` | monotonic root revision/checkpoint used for cache invalidation and read-after-write checks |
| `audit_events` | append-only qualified actor/token/session, scope/database UUID, action/object, decision/result, request/correlation ID, timestamp and bounded metadata |
| `migration_state` | package/migration checksums and applied timestamps, using existing managed-schema machinery |

Use UUIDv7 or another time-sortable random identifier when a suitable audited dependency is selected; UUIDv4 is acceptable initially. `DatabaseId` is specifically a UUID and never derived from a name, email, username, URI, or filesystem path. Normalize login identifiers deliberately and preserve display values separately.

Database names have one versioned grammar and normalization routine shared by config, CLI, HTTP, RPC, and migrations. For v1, accept 1–63 ASCII characters, lowercase ASCII before validation, and require `^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$`; reject leading/trailing whitespace rather than trimming it, non-ASCII/control characters, reserved names such as `root` and `system`, and any input that parses as a UUID. Preserve the accepted display spelling separately, but compare and select by the lowercase normalized key. Enforce global uniqueness across current names, live aliases, and tombstones with atomic unique indexes. Future Unicode support requires a versioned migration and cannot silently change normalization.

Renaming changes only the name record and descriptor generation, never `DatabaseId` or any grant/token/session/binding/audit/cache key. In v1, the old normalized name becomes an alias resolving to the same UUID for a configurable window (default 30 days), then becomes a non-resolving tombstone; current names, aliases, and tombstones cannot be reused. A deleted database's UUID and names are never reused. A future privileged tombstone-release feature would need its own ADR, retention checks, explicit audit, and tests; it is not part of v1. Selector resolution returns one generic `database_not_found_or_forbidden` for absent, unauthorized, expired/tombstoned, or otherwise non-resolvable input and must never guess among exact/current/alias matches.

### 5.3 Database-local identity and policy schema

Install a versioned, protected `semantic.local_auth` package in every multi-tenant database; per-database configuration may disable issuance of local credentials while retaining local policy and global bindings. It is accessed through typed repositories, excluded from normal catalog/query/export paths unless a specific local-auth administration capability is present, and migrated with the database. Suggested records:

| Collection | Key fields and purpose |
|---|---|
| `local_accounts` | local account ID, status, display metadata, security version, timestamps; identity is always paired with the containing database UUID |
| `local_login_identities` | local account ID, kind, normalized/display value, verified state; uniqueness is within this database only |
| `local_password_credentials` | local account ID, PHC Argon2id verifier, pepper key/version and rehash metadata |
| `local_tokens` | token ID, local principal, digest, database UUID audience, action ceiling, status/expiry/revocation metadata |
| `local_auth_sessions` | local session ID, digest, CSRF/auth metadata, immutable database UUID audience, expiry/status |
| `local_groups` / `local_group_memberships` | database-owned groups and nested membership |
| `external_subjects` | root-attested global principal ID to credential-less local shadow subject, binding/elevation kind, generation/status and local profile metadata |
| `local_authz_models`, `local_relation_tuples`, `local_policy_bindings`, `local_authz_revisions` | database-domain policy state and monotonic revisions |
| `local_audit_events` | local authn, account, group, binding, policy, decision and data-access events with canonical database UUID |

Local database administrators may create and manage local accounts, credentials, groups, and policies only through typed APIs. They cannot write binding/elevation attestations, edit the required platform-elevation rule in the protected base model, manufacture `global:*` subjects, grant root entry, or create a token for another database. A local backup therefore contains its local security configuration, but restoring it under a different registered UUID must disable/quarantine local credentials, tokens, sessions, and external bindings until an explicit re-key/rebind operation completes.

### 5.4 Transaction and integrity prerequisite

Global/local account creation, unique identity and database-name assignment, token rotation, tuple mutation plus revision increment, and database provisioning require atomicity in the owning store. `SemanticDb::execute_batch` is useful but the current public trait does not expose explicit conditional writes/isolation. Before relying on generic root or local-auth stores:

1. Define each owning store's required capabilities: atomic multi-record batch, unique index enforcement, compare-and-set/revision conflict, durable commit, and consistent read for authorization changes.
2. Add capability probes and backend conformance suites for both root control and protected database-local auth repositories.
3. Either extend `SemanticDb` narrowly with the missing guarantees or implement backend-specific `ControlPlaneStore` adapters. Do not emulate uniqueness with an unlocked query-then-insert sequence.
4. Treat unsupported backends as a startup error in multi-tenant mode.

This is a correctness and security gate, not an optimization.

### 5.5 Bootstrap and recovery

- Add a non-network bootstrap command, for example `semantic-server bootstrap-admin --root-db ...`, which creates the first platform-admin account and either reads a password from a TTY/stdin or emits one single-use recovery token exactly once.
- Never accept a default admin password or print reusable credentials during normal startup.
- Make bootstrap idempotent: it refuses to create another first admin unless invoked with an authenticated recovery procedure.
- Record a bootstrap audit event and require credential rotation after use of a recovery token.
- Provide `migrate-root`, `check-root`, `rotate-token-pepper`, qualified global/local `disable-account` and `revoke-token`, `reconcile-bindings`, and database-name integrity operational commands that work without starting the public server.
- Protect break-glass recovery material outside the database; document quorum/operator procedure for production.

## 6. Accounts, credentials, tokens, and sessions

### 6.1 Principal model

Replace request use of the bare `Principal` with an immutable authentication result:

```text
AuthenticatedPrincipal
  principal_ref = Global(global_principal_id)
                | DatabaseLocal { database_id, local_principal_id }
                | LocalSingleUser (auth-less mode only)
  principal_kind
  account/service status snapshot
  authentication_method + authentication_time + assurance level
  auth_session_id/token_id (one or neither for internal workloads)
  canonical UUID database audience and hard scope/action ceiling
  principal security_version / credential_version
  platform capability/elevation marker derived from policy, not client claims
```

Keep `Principal::system()` only for trusted in-process initialization/background jobs. Constructing it should be impossible from network headers, token claims, database-local data, or request payloads. `LocalSingleUser` is also a separate non-serializable server construction; it is not `system`, `platform_admin`, or a database-local account and exists only under the auth-less mode's fixed-main-database capability.

Global and local account namespaces never merge automatically. A matching username, email, external-provider address, or account ID is not evidence that two accounts are the same. API responses and audit events expose the identity domain where authorized, and every `SubjectRef` serialization includes `global:` or `database:<uuid>:`.

### 6.2 Passwords

- Store only PHC-encoded Argon2id verifiers with per-password random salts; keep cost parameters in the PHC string and rehash after successful login when policy changes.
- Benchmark parameters on deployment hardware and start no weaker than the RFC 9106 memory-constrained recommendation unless a documented operational benchmark requires adjustment. RFC 9106 recommends Argon2id and provides 2 GiB/one-pass and 64 MiB/three-pass profiles ([RFC 9106](https://www.rfc-editor.org/rfc/rfc9106.html)).
- Use a server-side pepper stored outside the database containing the verifier when operational secret management is available; version pepper keys for rotation. Root and database-local stores may use separately scoped pepper keys so compromise of one database does not expose every verifier.
- Run password hashing on a bounded blocking pool with concurrency limits so login attempts cannot exhaust the async runtime or memory.
- Apply length limits, breached/common-password screening, rate limits, and uniform failure responses. Never log passwords or retain them in general `String` values longer than necessary.
- Design credential traits for future WebAuthn/OIDC/MFA; platform-admin production policy should eventually require phishing-resistant MFA.

### 6.3 Opaque token format and storage

Use a structured opaque token such as:

```text
sem_gat_<base64url token_id>.<base64url 32-byte random secret>
sem_gpat_<base64url token_id>.<base64url 32-byte random secret>
sem_lat_<base64url database_uuid>.<base64url token_id>.<base64url 32-byte random secret>
sem_lpat_<base64url database_uuid>.<base64url token_id>.<base64url 32-byte random secret>
```

The prefix distinguishes the global/local identity domain and short access vs personal/service token. The visible database UUID in a local token is a routing hint, not authority; after parsing it, the server resolves that exact registered database, opens only its protected local-auth repository, verifies the secret, and requires the stored UUID audience to match. Names are not embedded in tokens because they can be renamed. Persist only `token_id` plus a constant-time-verifiable HMAC/digest of the secret, pepper key ID, metadata, canonical UUID audience, allowed scope/action ceiling, and expiry. Display the complete token once. Token lookup by public ID avoids scanning hashes; 256 random secret bits make offline guessing infeasible, while an external pepper adds database-compromise defense.

Token classes:

- short-lived access token, default lifetime measured in minutes;
- refresh/login session credential, rotated on use with reuse detection;
- personal access token with explicit name, expiry, database/action ceiling, and last-used time;
- service token bound to a service principal and audience.

All tokens are revocable in their owning identity store. Account disable, password reset, privilege-sensitive account changes, and `security_version` increments revoke or invalidate affected tokens/sessions. A local token can never select another database even if its request includes a valid different name/UUID; reject the conflict before data access. RFC 6750 requires protecting bearer tokens in transit, recommends short lifetimes and audience/scope restriction, and says not to put them in page URLs ([RFC 6750](https://www.rfc-editor.org/rfc/rfc6750.html)). Token revocation semantics should align with [RFC 7009](https://www.rfc-editor.org/rfc/rfc7009.html), and the internal verification result should carry the active/expiry/audience/scope fields described by [RFC 7662](https://www.rfc-editor.org/rfc/rfc7662.html).

Do not implement OAuth authorization-code flows in the first increment merely to issue local tokens. If third-party clients or external identity providers are added, implement a standards-compliant OAuth/OIDC profile and follow the current OAuth Security BCP, including authorization-code flow, PKCE, exact redirect matching, and replay defenses ([RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html)).

### 6.4 Browser and native sessions

Distinguish persisted authentication from per-connection selection:

- Browser login creates a random, rotated `AuthSession` cookie with `HttpOnly`, `Secure`, and an appropriate `SameSite` setting. Store only its digest in the owning root or local identity store. A local-session credential contains an integrity-protected database UUID routing component; the UUID is rechecked against the stored immutable audience. Add idle and absolute expiry, explicit logout, session listing/revocation, and fixation prevention on login/privilege changes.
- Cookie-authenticated state-changing requests require a CSRF defense (same-origin checks plus a CSRF token/header); CORS must be an explicit allowlist with credentials, never reflective `*`.
- Native/API clients send `Authorization: Bearer ...` and do not put tokens in query strings.
- An HTTP request may use either a bearer token or a session cookie according to an explicit precedence rule. Reject ambiguous requests that present conflicting valid identities.
- `ConnectionSession` retains only active auth scope/database and transport state. It references the authenticated token/session and cannot widen its scope ceiling.

OWASP emphasizes that a session identifier is temporarily equivalent to the user's strongest authentication and recommends secure generation, renewal, expiry, and cookie controls ([OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)).

### 6.5 Login and authentication routing

Expose separate intent even if one UI fronts both flows:

```text
POST /api/v1/auth/global/login
  { login, password }

POST /api/v1/auth/local/login
  { database: DatabaseSelector, login, password }
```

For local login, canonicalize the supplied UUID or globally unique name through the root registry before credential verification. The selector is only an untrusted routing hint. Open the selected database's protected auth interface, verify the local identity, and issue a principal/token/session permanently bound to the canonical database UUID. Use uniform outward errors and rate-limit by source plus a privacy-preserving selector/login key so name/account existence is not disclosed. A disabled/suspended database fails closed without consulting arbitrary user collections.

Global login consults only the root identity store. Database selection can happen after global authentication; presenting a database selector at global login may set a requested default but cannot grant entry. If one generic `/auth/login` endpoint is retained, require an explicit `identity_domain = global | database` field; never guess the domain by trying root and local password databases in turn.

Bearer/cookie routing parses only bounded, validated token metadata before selecting a verifier. Invalid prefixes, UUIDs, oversized IDs, conflicting selectors, and unknown databases use generic failures. Never scan every open database for a matching local token or accept a database name from a token as stable identity.

### 6.6 Global-to-local binding and lifecycle consistency

Each active global principal/database relationship has a root record and a corresponding local external subject (normally a credential-less shadow account). The local record carries the global ID, root binding generation, local subject ID/status, and database-specific profile/role/group relationships. A database may instead evaluate a typed external subject directly, but it still requires a root-attested active binding and domain-qualified ID.

Provisioning spans two databases and therefore is an idempotent saga, not a claimed distributed transaction:

1. Atomically create/update the root entry grant and binding intent as `pending`, with an idempotency key/generation.
2. Open the database through the registry, upsert the shadow subject and initial local relationships from an explicit database auto-provisioning policy, and record the generation.
3. Mark the root binding/grant `active` only after local verification; access fails closed while pending or inconsistent.
4. A durable outbox/reconciler retries interrupted work and reports orphaned root/local records. Replaying a generation is harmless.

Root entry permission alone never supplies local object rights. A database may explicitly configure safe auto-provisioning such as assigning new global members a `reader` role; absent that policy, a local administrator must grant local rights. Local policy alone cannot create root entry.

Revocation closes the outer boundary first: disable the root grant/binding, bump the root security/grant revision, invalidate global sessions/connections/caches, then disable/remove the local shadow asynchronously. Local-account revocation is atomic within the database and closes its database-bound sessions/tokens. Deleting/suspending a database invalidates both classes through registry state and UUID audience checks. Reconciliation and audit make delayed cleanup visible without preserving access.

## 7. Auth scopes and database access

### 7.1 Resolution algorithm

Resolution is identity-domain aware.

For a global credential:

1. Authenticate against the root identity store and verify status, expiry, audience, security version, and revocation.
2. Collect the optional auth-scope selector and optional `DatabaseSelector` from the command/envelope/connection/token defaults. Resolve an explicit UUID-or-name database selector through the root registry to exactly one active canonical `DatabaseId` before using it as a policy key.
3. Resolve the active auth scope within the token's hard ceiling. Prefer an explicitly requested scope, then a still-authorized token/session default. If neither exists but a database was selected, derive the scope only when exactly one enterable scope grants that UUID; zero candidates yields generic denial and multiple candidates yields `auth_scope_required` rather than an arbitrary choice. If no database was supplied, use the resolved scope's default database UUID. Verify `auth_scope.enter` and recheck that the final UUID is in the scope.
4. Evaluate the root-domain `database.connect` and command boundary for that UUID. This is the non-overridable outer ceiling.
5. Resolve and validate the active global-to-local binding/shadow subject at the required generation.
6. Acquire a `DatabaseLease`, evaluate the database-local command/object policy for that local/external subject, and execute through `AuthorizedDb` only if both decisions allow.

For a database-local credential:

1. Use its bounded UUID routing component to locate the registered database and its protected verifier; authenticate and verify the stored immutable UUID audience, local account status/security version, expiry, and revocation.
2. Canonicalize any request selector. It must resolve to that same UUID; a missing selector implies the token's UUID. Local principals cannot choose an auth scope or another database.
3. Require active registry/database state, acquire a lease, and evaluate the database-local policy. The root layer supplies only the structural boundary that this credential may address its registered origin; it does not give the local account a root membership.

In `single_user_authless`, skip both identity stores and selectors: construct the fixed `LocalSingleUser` capability for the configured `main` database and never expose database/auth-scope switching. This branch is selected at startup, not per request.

The normal composition rule is:

```text
global effective allow = root_boundary_allow AND local_policy_allow
local effective allow  = exact_database_audience AND local_policy_allow
auth-less allow        = server_mode_capability AND main_database_match
```

Never union root and local allows. Deny, indeterminate, stale binding/revision, unavailable policy storage, or mismatch at either applicable gate fails closed. An invalid or unauthorized UUID/name should normally return the same `database_not_found_or_forbidden` response to avoid existence enumeration. Inventory administrators may receive more detail only after authorization.

### 7.2 Selection precedence and protocol representation

Add explicit selection metadata to the RPC protocol rather than requiring every payload to understand `scope_id`:

```text
RpcRequestContext {
  auth_scope_id?: string,
  database?: DatabaseSelector, // UUID or globally unique name
  idempotency_key?: string,
}
```

During compatibility, accept `database_id`, `database_name`, the existing payload `scope_id`, `X-Semantic-Database`, `X-Semantic-Scope`, and `?database=`/`?scope=` as **database selectors**. Recommended precedence is command-explicit selector, RPC envelope, connection current UUID, auth-scope default UUID. Resolve every supplied UUID/name independently, then reject them if their canonical UUIDs conflict; UUID and name forms resolving to the same database are equivalent. Do not retain the spelling/name beyond diagnostics. Local credentials reject any resolved UUID other than their audience.

Parser behavior is deterministic: typed API forms should use `{ "id": "<uuid>" }` or `{ "name": "<name>" }`; compact strings parse valid UUID syntax as ID and otherwise as name. Creation rejects names with UUID syntax. Apply the one normalization routine before the unique lookup, perform authorization before returning database metadata, and use ambiguity-safe generic errors for missing, tombstoned, duplicate-corrupt, or forbidden records. Duplicate normalized records are an integrity/readiness failure, never a reason to pick one.

Do not put credential material in this envelope. HTTP credentials remain headers/cookies; WebSocket identity is bound at connection authentication.

### 7.3 Admin semantics

- Split `platform_operator` (registry/health/lifecycle without tenant reads), `platform_security_admin` (global identities/root grants), `platform_support`, and `platform_data_admin`; do not use one ambient superuser bit.
- A `platform_data_admin` can obtain access to every known active database, satisfying the global administration requirement, but does so through an explicit database-bound, short-lived elevation. The elevation creates a root-attested local external capability, requires recent/reinforced authentication and a reason/ticket, and is evaluated by the database domain through a small protected base-model rule that grants only the elevation's explicit verbs. Database-specific policy still applies outside that verb ceiling; local administrators cannot turn an ordinary global account into this reserved subject or remove the platform safety hook.
- A separately named break-glass capability may bypass an unavailable/corrupt/misconfigured local gate only if the deployment explicitly enables it. Require strong recent authentication/MFA, reason/ticket, very short expiry, single-database UUID audience and explicit verbs, immutable root and local audit events, alerts, and post-event review. It cannot query the root DB, expose secrets, open unregistered URIs, or become `system`.
- Local `database_admin` can manage local accounts, groups, bindings' local role assignments, and policy within its database, but cannot activate a root binding, grant cross-database entry, administer global credentials, or issue break-glass capabilities.
- Emit immutable audit records for database listing, connection, policy changes, token creation/revocation, impersonation/support access, and destructive lifecycle actions.
- Keep a narrower platform database-provisioner role for registry/provisioning without identity/policy administration.

### 7.4 Policy ownership and change rules

Root policy owns global account status, auth-scope membership, database discovery/entry, cross-database ceilings, platform capabilities, binding/elevation validity, and break-glass issuance. Database policy owns local accounts/groups, local administration, and database/object verbs for local and ordinarily bound global subjects, plus a protected minimal rule for valid platform elevation. Local custom policy may be stricter for ordinary subjects but never relax the root decision or forge reserved platform subjects. A root administrator may revoke entry immediately but should not silently rewrite ordinary local roles; a local administrator may revoke ordinary local binding/roles immediately but cannot preserve access when root revokes it.

Every authorization context carries `identity_domain`, canonical database UUID, root policy/grant/binding revision where applicable, local policy revision, and credential security version. Decision and subject caches include all of these. Renaming a database changes no authority; binding/grant/policy changes invalidate the relevant revisions.

## 8. Database registry and handle lifecycle

### 8.1 Persistent descriptor model

Store normalized descriptors, not arbitrary URLs returned to clients:

```text
DatabaseDescriptor
  database_id: UUID (immutable)
  database_name + normalized_name (globally unique selector)
  owner_tenant_id
  provider_kind
  locator (server-controlled path/name, not a user URI)
  credential_ref (secret-manager reference, never returned)
  open mode / read-only flag
  status: provisioning | active | suspended | deleting | deleted | error
  schema/migration target
  blob-store descriptor ID
  generation
  created/updated/deleted metadata
```

Registry service APIs accept `DatabaseSelector` at their boundary and return records keyed by UUID. All internal provider/open/lease calls accept `DatabaseId` only. Registration atomically reserves both UUID and normalized name; rename atomically swaps the current normalized name, writes history/reservation, increments descriptor generation, and publishes invalidation. Provider locators and filesystem names are independent of the public database name, so rename does not move storage.

Provider-specific validators canonicalize local paths under configured roots, reject root/control paths and symlink escapes, allowlist remote hosts/options, and prevent credential injection in URIs. The server constructs the final `DbOpenRequest`; ordinary users never submit one.

Provisioning uses a durable state machine and idempotency key. Record intent, create/open storage, migrate it, initialize required packages, bind object storage, mark active, and compensate/mark error on failure. Deletion is two-step (`suspended` then delayed purge), requires no active lease, and is recoverable until the retention deadline.

### 8.2 Handle manager

Refactor `ScopeManager` into a registry-backed `DatabaseManager`:

- key cache entries by `(DatabaseId, descriptor_generation)`, not principal;
- singleflight concurrent opens so one physical redb path is opened once;
- return RAII `DatabaseLease` values that increment/decrement active use;
- maintain `Opening`, `Ready`, `Retiring`, and `Failed(backoff)` states;
- enforce global/per-provider/per-tenant open-handle and connection-pool budgets;
- use idle LRU/TTL retirement only when lease count is zero;
- invalidate/reopen on descriptor generation changes;
- never hold a synchronous `RwLock` across `.await` or backend open;
- expose graceful shutdown/drain and bounded open timeout;
- use a shared pool per PostgreSQL descriptor and account for the deployment's finite connection budget; PostgreSQL allocates resources according to `max_connections` ([PostgreSQL connection settings](https://www.postgresql.org/docs/current/runtime-config-connection.html#GUC-MAX-CONNECTIONS)).

Opened-handle cache keys, object-store keys, catalog caches, query caches, rate limits, jobs, and audit records must all include the UUID `DatabaseId` and, where relevant, identity domain, `AuthScopeId`, root revision/binding generation, and local policy revision. Names may be bounded display labels in diagnostics but never cache/security keys. Never cache an authorization decision using only principal and object ID.

### 8.3 Background maintenance

Spawn supervised tasks for idle retirement, expired session/token cleanup, audit flushing, provisioning reconciliation, and policy-cache invalidation. Give each task bounded queues, cancellation, health reporting, retry/backoff, and tenant-aware fairness. One tenant's slow DB must not block registry/root operations or other tenants.

## 9. Permission model (`semantic_authz`)

### 9.1 Canonical check

Although product language may say “object-verb-subject,” standardize the internal check as:

```text
Can SUBJECT perform VERB on OBJECT in AUTHORIZATION_DOMAIN
    [and optional AUTH_SCOPE] with CONTEXT?
```

Example:

```text
subject = global:user:019...a
verb    = document.edit
object  = document:019...f (database db:019...d)
domain  = database:019...d
scope   = auth_scope:019...s (global request only)
context = { request_time, network_zone, authentication_level }
```

The application normally evaluates a global request twice: first in `Root` against the database UUID and global subject, then in `Database(uuid)` against the root-attested external/shadow subject. A local request is evaluated only in `Database(uuid)` after its UUID audience is established. Each `Decision` identifies its domain and revision; a `CompositeDecision` records both applicable outcomes and implements explicit intersection. The evaluator cannot read tuples from another domain unless the caller supplies a separately validated, typed external-subject binding.

The evaluator is default-deny. Missing data, unknown verbs/types, storage errors, stale required revisions, invalid policy, cycles beyond supported semantics, or budget exhaustion produce a deny/indeterminate result that enforcement treats as denial.

### 9.2 Relation-based core

Use Zanzibar-inspired relation tuples:

```text
root:database:<uuid>#editor@root:auth_scope:<scope-id>#member
database:<uuid>:document:doc1#owner@database:<uuid>:user:<local-user-id>
database:<uuid>:document:doc1#viewer@database:<uuid>:group:researchers#member
database:<uuid>:document:doc1#owner@database:<uuid>:external:<binding-id>
database:<uuid>:folder:team#parent@database:<uuid>:folder:company
```

Tuple parsing requires fully qualified subject/object domains. A database-domain store rejects tuples naming a different database UUID and accepts a `global:*` reference only through its protected external-subject record whose binding generation is attested by the root service. Thus a local admin cannot type a global admin ID into a tuple and gain root authority.

The model defines relations and permission rewrites using direct relation, union, intersection, exclusion (only if deliberately supported), computed userset, and tuple-to-userset/parent traversal. Zanzibar demonstrates how object-user and object-object relation tuples plus usersets model nested groups and inherited access; it also highlights why permission checks need explicit consistency semantics after ACL changes ([Google Zanzibar paper](https://research.google/pubs/zanzibar-googles-consistent-global-authorization-system/)).

### 9.3 Dynamic and scoped subjects

Support these subject forms, all typed and validated:

- concrete user/service/internal principal;
- group userset such as `group:eng#member`;
- root-domain auth-scope userset such as `auth_scope:x#member`;
- tenant userset such as `tenant:x#admin`;
- object relation userset such as `folder:x#viewer`;
- bounded dynamic selector referencing trusted subject/object/environment attributes.

Dynamic subjects are not arbitrary scripts, SQL fragments, closures, or plugin callbacks. Define a versioned condition AST with an allowlisted operator/type system: equality/set membership, ordered comparison, boolean composition, CIDR/time-window checks, and references such as `subject.id`, `subject.attributes.department`, `object.attributes.owner`, `scope.id`, and server-supplied `context.authentication_level`. Attribute providers declare freshness and provenance. Clients may supply only explicitly public context keys; security-sensitive values (time, source address, auth method, tenant/scope) are overwritten by the server.

This combines ReBAC with constrained ABAC. NIST defines ABAC decisions in terms of subject, object, operation, and sometimes environment attributes evaluated against policy ([NIST SP 800-162](https://csrc.nist.gov/pubs/sp/800/162/upd2/final)).

### 9.4 Hierarchy

- Represent hierarchy through explicit typed relations such as `parent`, not string-prefix IDs.
- A model opts a permission into propagation, for example `viewer = direct_viewer or parent.viewer`; inheritance is never implicit for every verb.
- Validate parent edges according to the object's declared graph kind. Enforce a DAG when required; otherwise allow cycles in stored graphs but guarantee visited-set detection and a strict traversal budget.
- Bound maximum depth, node fan-out, total datastore reads, condition evaluations, and wall time. Return `indeterminate_budget_exhausted`, treated as deny.
- Version model changes and tuple writes. A mutation returns an authorization revision; callers can request `at_least(revision)` to prevent “revoked but still visible” races.

### 9.5 Policy lifecycle and explanations

- Models are immutable after activation; edits create a draft version, validate it, run fixtures/shadow checks, then atomically switch the root scope's or database domain's active model.
- Store policy tests alongside model drafts: positive and negative checks, especially cross-tenant cases.
- `Decision` includes a stable reason/rule ID and dependency revision. Full graph explanations are privileged, bounded, redacted, disabled by default, and never returned to an unauthorized caller because they can reveal relationships.
- Cache only complete decisions or safe intermediate usersets with keys containing authorization domain/database UUID, model revision, tuple revision, root binding/grant revision when applicable, scope, qualified subject, verb, object, and relevant context fingerprint. Revocation/policy updates publish invalidations; high-risk/admin checks can require uncached/current reads.
- Do not add explicit deny rules in the first model. Unioned grants plus default deny are easier to reason about. If deny is later required, specify conflict precedence formally and add exhaustive tests before enabling it.

## 10. Enforcement integration

### 10.1 One authorized context

Replace freely constructed `AppRequestContext` fields with mode-specific constructors. Multi-tenant construction accepts a verified domain-qualified `AuthenticationContext`, canonicalizes `DatabaseSelector` to UUID once, and produces an `AuthorizedRequestContext` after applicable root/binding/local checks. Auth-less construction accepts only the startup-owned fixed-main capability. Keep fields private so tests and commands cannot manufacture authorization or change identity domains.

Expose methods such as:

```text
context.authorize(verb, object)
context.database(required_verb) -> DatabaseLease / AuthorizedDb
context.object_store(required_verb) -> AuthorizedObjectStore
context.require_platform_action(verb)
context.identity_domain() -> Global | DatabaseLocal(DatabaseId) | LocalSingleUser
```

The database manager must require a previously issued internal authorization grant/capability containing UUID, principal domain, applicable root/local revisions, and mode, not just `DatabaseId`, to reduce confused-deputy mistakes. No lower layer accepts `DatabaseName` or raw selector strings.

### 10.2 Command metadata

Extend the app command registry with mandatory authorization metadata:

- static command verb;
- target resolver from validated payload plus selected DB;
- whether object prefetch is needed;
- optional result filter strategy;
- whether recent authentication/admin reason is required;
- audit classification and sensitivity.

Registration fails if a non-public command lacks policy metadata. In multi-tenant mode, authentication/login/health commands are explicitly marked public rather than implicitly bypassed. Auth-less mode has its own route/command registry whose data operations require the private main capability even though they require no credential. Keep `semantic_rpc::RpcCommand` transport-neutral; wrap it with an app-level `AuthorizedCommand` adapter instead of coupling the RPC crate to authz.

Initial verb map:

| Existing command/path | Initial required permission |
|---|---|
| `semantic.scope.list/current/use` | authenticated; list only enterable scopes/databases |
| `semantic.scope.open` | deprecated; `database.register/open_uri` platform DB admin only |
| `semantic.db.catalog`, `query`, `get` | `database.read` initially |
| `semantic.db.insert`, `delete`, `batch`, `package.upsert` | `database.write` or narrower admin verb |
| file upload/analyze | `file.create`/`file.analyze` in selected database |
| file download | `file.read` on the resolved file object |
| account/token/session methods | self action or `identity.manage` |
| local account/token/group/policy methods | database-domain self action or `local_identity.manage`; fixed to selected UUID |
| tenant/scope/database/policy APIs | namespaced admin action |

### 10.3 Object-level authorization and query safety

Object-level authorization cannot be declared complete while arbitrary query APIs can bypass it. Deliver it in this order:

1. Phase 1 authorizes whole databases. A user granted `database.read` may query all records in that DB; otherwise every DB command is denied.
2. Introduce `AuthorizedDb` and object-aware `get`/insert/update/delete methods. Resolve a stable `ObjectRef` from database, collection/class, and entity ID; check before reads and writes. For owner-dependent create policy, validate the proposed object and check both collection create and assigned relationships.
3. Add authorization-aware query planning that injects mandatory visibility predicates/candidate-ID sources before scan, join, aggregate, sort, pagination, or mutation. Enforcement belongs below all text-query/AST entry points.
4. Until a query shape can preserve authorization semantics, require a database-wide `database.query_unrestricted` permission or reject it. Never post-filter final rows: aggregates, counts, ordering, limits, errors, timing, and joins can leak unauthorized data.
5. Batch operations authorize every target and execute only if all checks succeed unless the API explicitly defines a per-item partial result. Avoid check-then-use races by carrying a required authz revision into the transaction where possible.

PostgreSQL RLS can be defense in depth for a future shared-table backend, but table owners and `BYPASSRLS` roles bypass it unless configured carefully; policy subqueries can also create concurrency races ([PostgreSQL Row Security](https://www.postgresql.org/docs/current/ddl-rowsecurity.html)). The application-level permission engine remains authoritative and separately tested.

### 10.4 File/object-store enforcement

- Resolve file metadata inside the already authorized database, then authorize the file object before opening the blob.
- Partition blob locators by database/tenant and verify the returned metadata matches the selected database.
- Cookie-authenticated same-origin file URLs are suitable for browser rendering. For external/direct storage access, mint short-lived signed file capabilities containing file/database, verb, audience, expiry, nonce, and optional disposition; do not reuse general API tokens in URLs.
- Authorize ranges exactly as full reads; cap range/count/amplification; avoid existence-revealing errors.
- Include database/scope in object-store cache keys and asynchronous media-analysis jobs.

## 11. HTTP, WebSocket, RPC, and client changes

### 11.1 HTTP API

Add normal HTTP semantics around the existing RPC body in multi-tenant mode:

- `POST /api/v1/auth/global/login` and `/api/v1/auth/local/login` (the latter requires a UUID-or-name `DatabaseSelector`), plus `/refresh` and `/logout` routed to the authenticated identity domain;
- `GET /api/v1/auth/me`, `/sessions`, `/tokens`;
- `POST /api/v1/auth/tokens`, token/session revoke endpoints;
- typed local administration endpoints for accounts, credentials, tokens, groups, external-subject bindings' local assignments, and database policy; every request is pinned to one canonical UUID;
- typed admin RPC or REST endpoints for tenants, scopes, database registry/provisioning, memberships, models, and tuples.

Database registration responses include both UUID and current name. Selection-taking endpoints accept the typed `DatabaseSelector`; grants, token metadata, session state, audit events, and binding responses expose/store the canonical UUID. Rename is a distinct conditional admin operation with expected generation, collision/tombstone checks, audit, and no change to active sessions or grants.

In `single_user_authless`, do not mount login, token, session, tenant, registry mutation, cross-database selection, global identity, local identity administration, or break-glass routes. Existing database RPC/file routes target `main` implicitly. By default any supplied selector is rejected as `selection_not_supported`; an explicit compatibility flag may accept it only after verifying it equals the persisted main UUID/current name, and it is never ignored or used to open storage.

Authenticate middleware before tenant/database resolution. Use `401` plus `WWW-Authenticate: Bearer` for absent/invalid/expired credentials, `403` for known but insufficient permission where disclosure is safe, `404`/generic denial for tenant-owned resource enumeration, `409` for state/version conflicts, and `429` for throttling. Preserve `RpcResponse.id` and stable machine error codes.

### 11.2 WebSocket

- Browser sockets authenticate at upgrade with the same-origin secure session cookie. Browser WebSocket APIs cannot generally attach an arbitrary Authorization header, so do not place bearer tokens in the WebSocket URL.
- Native clients use an upgrade `Authorization` header through a request-builder API. Optionally support a first-message challenge/auth protocol only if cookie/header authentication cannot meet a target client; it must time out and reject all other frames until complete.
- Validate `Origin` for browser upgrades, cap frame/message sizes and in-flight requests, implement ping/idle/absolute lifetimes, and apply per-principal/tenant connection limits. Follow OWASP's WebSocket guidance for origin validation, authentication, authorization per message, size/rate limits, and logging ([OWASP WebSocket Security](https://cheatsheetseries.owasp.org/cheatsheets/WebSocket_Security_Cheat_Sheet.html)).
- Bind the socket to qualified principal/identity domain, credential ID, security version, auth scope if global, and canonical database UUID. Revalidate root grant/binding and local revisions as applicable and react to revocation/security-version broadcasts by closing with a documented application close code.
- Serialize state-changing connection commands (`auth_scope.use`, `database.use`) relative to subsequent messages. The current spawn-per-message implementation can otherwise race selection changes. Ordinary independent requests may still execute concurrently from a captured immutable selection snapshot.
- Never allow a per-message payload to change the authenticated principal.

### 11.3 RPC clients

Add a redacting `CredentialProvider`/`RpcClientOptions`:

- native HTTP attaches `Authorization` and selection headers;
- browser HTTP supports cookie credentials and CSRF header/token; an explicit in-memory bearer provider may be supported but must not default to local storage;
- native WebSocket builds an authenticated upgrade request;
- browser WebSocket uses cookies and origin;
- clients expose active auth scope/database (UUID plus current display name) as typed context, accept either UUID/name selection at the API edge, update canonical UUID only after server confirmation, and clear them on logout/revocation;
- uploads and file URLs use the same auth configuration as RPC.

Do not put secrets in `Debug`, clone them into error messages, or serialize them into application `Value` payloads unnecessarily.

## 12. Security and threat model

Authorization should follow deny-by-default and validate permission on every request; OWASP also recommends preferring attribute/relationship-aware models over role-only growth for multi-tenant systems ([OWASP Authorization](https://cheatsheetseries.owasp.org/cheatsheets/Authorization_Cheat_Sheet.html)).

| Threat | Primary controls | Required test/evidence |
|---|---|---|
| Scope/database IDOR | Authenticate first; canonicalize UUID/name; server-side root and local checks; generic denial | Full two-tenant matrix over HTTP, WS, files, every command and selector form |
| Name collision/rename/reuse confusion | One normalization routine; atomic global uniqueness; UUID-looking-name rejection; tombstones/aliases; UUID-only authority state | Unicode/case/normalization, concurrent create/rename, stale-name and reuse tests |
| Identity-domain spoofing | Qualified principal/subject types; local storage cannot attest global subjects; no username/email merging | Same identifiers across root/two DBs, forged global tuple/header, wrong-domain serialization tests |
| Local-token routing/audience swap | Bounded UUID routing prefix plus secret verification and stored UUID match; no DB scans; request selector must match | Mutated prefix, copied token row, renamed DB, alternate selector, restore-under-new-UUID tests |
| Unsafe root/local permission composition | Explicit root AND local composite decision; deny/indeterminate at either gate; versioned binding | Truth-table/property tests, root revoke with stale local allow, local revoke with root allow |
| Binding provisioning split-brain | Pending/active saga, generation/idempotency, outbox reconciliation, outer-first revoke | Failure injection at every saga step and orphan repair tests |
| Root DB disclosure | Physically separate URI/credential; no registry entry; typed root store only | Attempts through list/open/query/file APIs always fail |
| Raw URI/path/SSRF abuse | Admin-only provisioning; provider allowlists; canonical path containment; secret refs | Traversal, symlink, encoded path, host/option fuzz tests |
| Token theft/replay | TLS, short expiry, audience/scope ceiling, digest-at-rest, rotation/revocation, optional future DPoP/mTLS | Revocation and refresh-reuse concurrency tests |
| Token leakage | Redacting secret types; no URL tokens; header/cookie scrubbing in logs/traces/errors | Snapshot/log scanner and panic/error tests |
| Password cracking/DoS | Argon2id, external pepper, bounded hashing pool, login rate limits | Parameter benchmark and saturation test |
| Session fixation/CSRF | Rotate on login/privilege change; secure cookie; CSRF/origin checks; logout invalidation | Browser integration tests across all mutating endpoints |
| WebSocket hijacking/stale auth | Origin check; upgrade auth; per-message authz; revocation close; bounded frames/in-flight | Cross-origin, revoked socket, race, flood tests |
| Confused deputy/bypass | Private context fields; authorized DB capability; mandatory command metadata; no raw root/DB handles | Compile/API review plus registration tests |
| Object-query bypass | DB-wide gating until mandatory query rewrite; checks below every query form | Adversarial joins, counts, aggregates, update/delete tests |
| Policy graph explosion/cycle | Validation, visited set, depth/read/time/fan-out budgets, deny on indeterminate | Property/fuzz tests and pathological graph benchmarks |
| Stale authorization after revoke | revisioned writes/checks, invalidation, security versions, short caches | Read-after-revoke tests across instances |
| Cross-tenant cache/job/blob bleed | Composite keys and payload assertions include tenant/scope/database | Collision and poisoned-cache tests |
| Admin abuse/break-glass | Separate operator/security/data roles, database-bound elevation, recent strong auth, reason, short expiry, immutable dual audit and alerts | Admin negative-role, expiry/audience, bypass-disabled, alert and post-review tests |
| Enumeration | Uniform UUID/name/auth failures, authorization before metadata, generic tenant-object denial, rate limiting | Response/status/body/timing envelope tests |
| Noisy neighbor/resource exhaustion | per-tenant quotas/rate/concurrency/pool budgets; fair queues | Load test with one abusive tenant and one healthy tenant |
| Root/tenant backup mismatch | documented checkpoints, revision metadata, reconciliation tooling | restore drill and orphan/missing DB report |
| Auth-less server exposed or mode-confused | Explicit startup enum, loopback default, one fixed main DB, no auth/registry routes, incompatible-option rejection, no network-derived principal | startup matrix, route inventory, non-loopback warning/error, selector/spoof attempts |
| Local-auth collections queried as user data | protected system package, typed repositories, catalog/query/export exclusions, specific admin verbs | raw query/catalog/backup/export negative tests for non-admins |

Additional baseline requirements:

- Require HTTPS/WSS in multi-tenant production; support trusted-proxy headers only from configured proxy networks.
- Explicitly configure CORS, CSP/security headers, cookie domain/path, public origin, and maximum body/frame/upload sizes.
- Keep cryptographic algorithms/parameters configurable by version, not user input. If JWT support is added later, pin allowed algorithms and validate issuer, audience, token type, time claims, and key source according to [RFC 8725](https://www.rfc-editor.org/rfc/rfc8725.html).
- Use constant-time secret comparison and OS CSPRNGs; keep encryption/signing/pepper keys in a secret manager or permission-restricted file, with key IDs and rotation runbooks.
- Security events must never include passwords, token/session secrets, raw cookies, database credentials, or complete sensitive objects.

## 13. Configuration, operations, and observability

### 13.1 Configuration

Introduce a typed configuration tree with CLI/env/file precedence and validation. Candidate settings:

```text
SEMANTIC_SERVER_MODE=single_user_authless|multi_tenant
SEMANTIC_MAIN_DB_URI=...                 # auth-less only
SEMANTIC_MAIN_DB_NAME=main              # first initialization / asserted persisted name
SEMANTIC_BIND=127.0.0.1:3000            # auth-less safe default
SEMANTIC_ROOT_DB_URI=...
SEMANTIC_ROOT_DB_CREDENTIAL_REF=...
SEMANTIC_TOKEN_PEPPER_FILE=...
SEMANTIC_PUBLIC_ORIGIN=https://...
SEMANTIC_ALLOWED_ORIGINS=...
SEMANTIC_COOKIE_SECURE=true
SEMANTIC_ACCESS_TOKEN_TTL=...
SEMANTIC_SESSION_IDLE_TTL=...
SEMANTIC_SESSION_ABSOLUTE_TTL=...
SEMANTIC_DATABASE_ROOT=...
SEMANTIC_MAX_OPEN_DATABASES=...
SEMANTIC_MAX_OPEN_DATABASES_PER_TENANT=...
SEMANTIC_MAX_CONNECTIONS_PER_TENANT=...
SEMANTIC_AUTHZ_MAX_DEPTH/READS/DURATION=...
SEMANTIC_TRUSTED_PROXY_CIDRS=...
```

Model mode as an enum, not independent `AUTH_REQUIRED`/multi-database booleans. Validate unsafe combinations at startup: multi-tenant without root/auth/bootstrap; auth-less with root/global/local-auth/token options, registry databases, or cross-database selectors; auth-less non-loopback bind without an explicit high-friction unsafe override and startup warning; secure cookies on an inconsistent public origin; root and tenant DB path equality; missing pepper; wildcard credentialed CORS; or unbounded pools. Secret values must use redacting types and never appear in the effective-config dump. The server must never fall back from failed multi-tenant startup into auth-less mode.

On first auth-less initialization, persist a generated UUID and normalized configured name in the main DB's protected identity metadata. On later starts, configuration asserts rather than overwrites that identity: URI identity/UUID or name mismatch is a startup error requiring an explicit offline rename/adoption operation.

### 13.2 Health and readiness

- Liveness says the process/runtime is functioning.
- Multi-tenant readiness requires root DB connectivity, current root migrations, valid active root/local authz prerequisites, at least one usable admin/recovery path, and functioning required providers. Auth-less readiness requires only the configured `main` database/blob store and never probes a root DB.
- Report tenant database health only to authorized inventory APIs; public health must not enumerate databases.
- Expose reconciliation status for provisioning/deletion/migration jobs.

### 13.3 Logs, metrics, traces, and audit

Every request/decision should carry a random request/correlation ID. Structured fields may include qualified principal ID, token/session ID, tenant, auth scope, canonical database UUID, optional bounded current name for display, root/local decision reason and revision, command/verb, latency, and result, subject to privacy policy.

Avoid unbounded metric labels: use aggregate outcome/provider labels and optionally a bounded internal tenant tier; put raw IDs in access-controlled logs/traces instead. Add metrics for authentication outcomes, denied decisions, token/session revocations, DB open state/latency/cache hit/leases, pool saturation, authz evaluation depth/reads/cache, rate limits, audit queue drops, and per-tenant resource accounting.

Security audit is distinct from diagnostic logging: append-only, durable enough for the deployment, time-synchronized, access-controlled, retention-managed, and exportable. If the audit sink is unavailable, define per-action behavior: privileged policy/admin mutations should fail closed; ordinary reads may continue with a prominent degraded alert only if policy explicitly permits.

## 14. API and migration compatibility

### 14.1 Modes

- `single_user_authless`: preserve the simple local/default DB and embedded UI with exactly one configured database named `main`. Persist a UUID plus normalized/display name in protected database metadata so it already has stable identity, but do not require a root DB, account, token, or local-auth package. Bind to loopback by default. The server constructs a private `LocalSingleUser` capability scoped to that UUID; it never maps an external header/payload to `system`, global, or database-local account identity.
- In auth-less mode, startup opens only `SEMANTIC_MAIN_DB_URI`; database registration/open-URI, auth-scope switching, account/token APIs, external principal headers, and dynamic DB selection are not mounted. Requests omit a selector. For compatibility, an optional selector may be accepted only when it resolves locally to the persisted main UUID or normalized name; any other selector is rejected without opening anything.
- `multi_tenant`: requires the root DB, explicit global/local authentication, bootstrap state, validated public/security config, registry-backed UUID/name selection, and no auth-less capability.
- Mode is process-wide and immutable until restart. Do not infer it from database count, request fields, root availability, or authentication failure, and do not serve auth-less and authenticated routes from one listener/runtime.

Keep auth-less single-user as the default for one compatibility release if necessary, but print a clear mode/bind/database UUID+name banner with no secrets. Never allow no-auth multi-tenant mode. Non-loopback auth-less serving is unsafe and should require an explicit opt-in designed for trusted networks; recommend a firewall/reverse proxy or switching to multi-tenant auth instead.

### 14.2 Existing default DB

Provide an idempotent adoption command:

1. While auth-less serving is stopped, open/check the existing main DB without moving it and create/read its protected UUID/name identity metadata. Default the name to `main`; if the target root registry already reserves it, require the operator to choose another valid globally unique name while preserving the UUID.
2. Create/migrate the separate root DB, bootstrap global admin, tenant, and auth scope.
3. Register the exact existing UUID and chosen globally unique name, provider descriptor, and blob store; never mint a replacement UUID merely because the source was previously auth-less.
4. Install/migrate `semantic.local_auth`, create a root-attested shadow subject for the bootstrap global admin, assign an explicit local owner role, and complete the pending-to-active root binding saga.
5. Record descriptor/binding generations and migration status; verify UUID/name lookup, local package protection, global login, both authorization gates, open/read/write, and audit.
6. Emit a dry-run/migration report, then restart explicitly in `multi_tenant`. Do not translate the former `LocalSingleUser` capability into a reusable credential or expose the DB concurrently in both modes.

Do not copy or delete user data automatically. Before authenticated writes begin, rollback can stop the server and return to auth-less mode with the original path/UUID intact; installed protected auth metadata remains dormant. After global/local account or policy administration begins, require an explicit rollback assessment/export because auth-less mode bypasses those policies. The tool never silently falls back.

### 14.3 Deprecations

- Remove `semantic.scope.open` with raw URI from auth-less network routes and make it platform-provisioner-only in multi-tenant mode; replace it with `semantic.database.register/provision` and `semantic.database.use`.
- Continue accepting `scope_id` as a deprecated `DatabaseSelector` during a versioned RPC transition; add `auth_scope_id` and typed `database` immediately. `database_id` and `database_name` remain convenience input forms but responses always carry both current name and canonical UUID.
- Preserve existing command names where semantics are safe, but return new stable error codes (`unauthenticated`, `token_expired`, `session_expired`, `not_found_or_forbidden`, `insufficient_permission`, `scope_required`, `database_required`, `database_unavailable`, `authz_indeterminate`).
- Version the protocol/capability response so old clients can detect that authentication and separate auth-scope/database selection are required.

## 15. Phased delivery

Each phase should be independently reviewable and deployable behind explicit configuration/feature gates.

### Phase 0 — invariants, ADRs, and security harness

- Record identity domains, root/local policy ownership and AND composition, root isolation, UUID/name selector and rename/reuse rules, opaque-token routing, auth-less mode isolation, admin/break-glass, consistency, and query-safety decisions as ADRs.
- Add typed UUID `DatabaseId`, `DatabaseName`, `DatabaseSelector`, qualified principal/subject IDs, authorization domains, and compile-time/private-field boundaries without changing behavior.
- Build a global/local/single-user and two-principal/two-tenant negative test harness plus command/route authorization inventory.
- Add request IDs and secret-redaction tests.

Exit: every current network/data path is inventoried and has an intended authentication, database-resolution, and authorization hook.

### Phase 1 — root DB and persistent database registry

- Add `semantic_control_plane`, schema package/migrations, root store capability contract, and backend conformance tests.
- Open/migrate the root DB at startup; add offline bootstrap/check/migrate commands.
- Add tenants, auth scopes, UUID+globally-unique-name database descriptors, name history/tombstones, grants/binding intents, provisioning states, and audit skeleton.
- Refactor `ScopeManager` into registry-backed `DatabaseManager` with singleflight leases and quotas.
- Persist UUID/name identity metadata in auth-less `main`; preserve its root-free fixed-database path and build the explicit adoption dry-run/tooling.

Exit: registry and lifecycle survive restart; concurrent name create/rename is unambiguous; root DB cannot be selected; two registered databases are isolated operationally; auth-less mode runs with no root DB and can open only main.

### Phase 2 — accounts, authentication, tokens, and sessions

- Implement global accounts/login identities plus the protected `semantic.local_auth` package for per-database accounts, credentials, groups skeleton, tokens/sessions, and policy revisions.
- Implement Argon2id credentials, domain/routing-aware opaque access/PAT tokens, persisted root/local browser sessions, refresh rotation/reuse detection, disable/revoke, security versions, and separate global/local login APIs.
- Replace network use of `system`; add HTTP middleware, login/logout/me/token/session APIs, CSRF/CORS/origin handling, and rate limits.
- Add root binding intents, local shadow subjects, idempotent provisioning/reconciliation, and outer-first revocation.
- Add credential-capable HTTP/WS/file clients and basic UI login/logout.

Exit: HTTP, WebSocket, upload, and download reject missing/invalid/revoked credentials in multi-tenant mode; global and local accounts with the same login remain distinct; local credentials cannot leave their UUID; binding failure/retry and revocation work within the documented bound. Auth-less routes remain credential-free and mode-isolated.

### Phase 3 — validated auth scope and database selection

- Add separate `AuthScopeId` and UUID-or-name `DatabaseSelector` to request/session/RPC/UI, canonicalizing immediately to UUID.
- Enforce selector normalization/uniqueness/rename/tombstone rules, global scope entry/root grants, local token UUID audience, and binding state for every command and file path.
- Serialize WebSocket selection transitions and capture immutable per-request selections.
- Restrict raw URI opening to platform DB admins; add database/scope picker and admin registry APIs.

Exit: exhaustive cross-domain/account/scope/database and UUID/name tests pass; no selector or routing prefix grants authority; rename changes no active authority.

### Phase 4 — `semantic_authz` and coarse permissions

- Add the standalone crate, versioned model validation, tuple store/revisions, evaluator budgets, batch checks, cache keys/invalidation, and model tests.
- Express platform admin, tenant/scope membership, root database entry, local account/group, bound-global-subject, and database read/write/admin relationships through domain-separated engine stores.
- Add explicit composite decisions (`root AND local`) plus local-only UUID-bound checks; shadow-evaluate each gate and failure mode independently.
- Add mandatory command policy metadata and `AuthorizedRequestContext`/`AuthorizedDb` façade.
- Shadow-evaluate against Phase 3 grants, compare decisions, then make authz authoritative.

Exit: database-wide permissions use the extensible engine; unsafe union is impossible through public APIs; root/local revoke and revision tests pass fail-closed.

### Phase 5 — groups, hierarchy, and dynamic subjects

- Add root and database-local group/userset tuples, parent traversal, constrained typed conditions, attribute providers, explanation tooling, draft/validate/activate workflow, and model fixture APIs.
- Add policy administration with separation of duties and durable audit.
- Load/performance/fuzz test graph budgets and cache invalidation.

Exit: nested group, scoped subject, parent inheritance, condition, cycle, and revision semantics are specified and tested.

### Phase 6 — object-level enforcement and authorized query planning

- Add canonical object mapping for collections/entities/files.
- Enforce `get` and mutations through object checks; make batch behavior atomic and explicit.
- Integrate visibility constraints into logical/physical planning for supported query shapes.
- Gate or reject unrestricted text/aggregate/join/mutation queries until proven safe.
- Add optional PostgreSQL RLS defense in depth where applicable.

Exit: adversarial query suites demonstrate that unauthorized objects cannot affect returned data, aggregates, pagination, mutation counts, or observable errors.

### Phase 7 — production hardening and rollout

- Multi-instance root/local invalidation and revocation, external secret manager, backup/restore/rebind and saga reconciliation, platform-data-admin elevation and audited break-glass, admin MFA/external IdP option, quotas/fairness, dashboards/alerts, penetration test, and incident runbooks.
- Run shadow authorization and staged tenant canaries; demonstrate rollback.
- Switch production defaults only after compatibility telemetry and migration tooling are proven.

Exit: security review, operational game day, tenant-isolation load test, and restore drill pass.

## 16. Test and verification strategy

### 16.1 Unit and property tests

- typed UUID/name selector parsing/canonicalization, qualified identity serialization, and non-interchangeability;
- database-name normalization, UUID-looking-name rejection, global uniqueness under races, rename/alias/tombstone/reuse behavior, and Unicode/case edge cases;
- global vs per-database login normalization/uniqueness and account state transitions, including identical logins in several domains;
- global/local token format and routing, entropy source abstraction, constant-time verifier path, stored UUID audience, prefix mutation, rename stability, expiry/audience/scope ceilings, rotation/reuse/revocation;
- Argon2 parameter encoding and rehash detection;
- auth scope/database resolution precedence, equivalent UUID/name acceptance, conflict rejection, and local-principal fixed-database behavior;
- root/local decision intersection truth table, binding-generation validation, default denial, model validation, rewrite evaluation, group/hierarchy cycles, budgets, condition typing, missing attributes, revisions, and explanation redaction;
- binding saga idempotency and failure injection, outer-first revocation, shadow-subject spoof resistance, and reconciliation;
- database handle singleflight, lease retirement, descriptor-generation invalidation, failure backoff, quotas, and shutdown;
- secret `Debug`/serialization/log redaction.

Use property tests for evaluator determinism, irrelevant-tuple invariance, default denial, root/local AND composition, domain/revision/cache-key separation, selector normalization idempotence, cycle termination, and bounded evaluation. If explicit deny is ever introduced, do not assume monotonicity; define and test its precedence.

### 16.2 Backend conformance

Run the same security-critical store suite against root-control and database-local-auth adapters for supported redb and PostgreSQL backends:

- atomic multi-record commit/rollback;
- unique identities/memberships under concurrency;
- compare-and-set conflict and retry;
- durable token revocation and authz revision;
- atomic normalized database-name reservation/rename and tombstone enforcement;
- migration checksum mismatch behavior;
- crash/reopen recovery where the test harness permits.

### 16.3 End-to-end isolation matrix

For two tenants, global accounts, same-named local accounts in multiple databases, scopes, a service account, and platform/local database admins, test every transport and verb:

- correct access;
- other tenant's scope/database/object ID, UUID and normalized-name selectors;
- local credential against another UUID/name and global credential with missing/pending/revoked local binding;
- root allow/local deny, root deny/local allow, stale root binding generation, and unavailable root/local policy store;
- valid token constrained to another scope;
- disabled account/tenant/database;
- expired/revoked/rotated token/session;
- conflicting selector sources;
- database rename during sessions/requests, old alias/tombstone behavior, attempted name reuse, and UUID continuity;
- stale WebSocket after revoke or scope change;
- file upload/download/range/analyze;
- database query/get/insert/delete/batch/package/catalog;
- admin inventory and cross-tenant access audit;
- cache warm-up under tenant A followed by same IDs under tenant B.
- protected local-auth package query/catalog/export attempts and forged `global:*` local tuples;
- platform elevation and break-glass disabled/enabled, expiry, UUID audience, reason/MFA, dual-audit and alert behavior.

Generate the command/path matrix from registry metadata so adding a command without negative coverage fails CI.

Run a separate auth-less matrix: root URI absent/unavailable, fixed main UUID/name persistence, implicit main selection, rejection of other selectors/open/register/auth/admin routes, external principal/header spoof attempts, loopback default, unsafe-option validation, and an adoption dry-run/full upgrade/rollback rehearsal. Assert no root DB calls and no `system` construction from requests.

### 16.4 Security, fuzz, and load tests

- Fuzz bearer parsing, auth model/condition parsing, tuple IDs, RPC selection envelopes, provider descriptors, file metadata/range headers, and WebSocket frames.
- Test CSRF, CORS/origin, header smuggling/duplicates, cookie precedence, oversized requests, login timing envelope, path traversal/symlinks/SSRF, and error enumeration.
- Race account disable, token revoke, policy/tuple mutation, database suspend, session scope change, and concurrent requests.
- Benchmark evaluator depth/fan-out/cache and Argon2 pool saturation.
- Load one abusive tenant beside a normal tenant and prove rate, connection, DB-handle, hashing, query, upload, and background-job fairness.
- Add dependency/license/advisory scanning and an external security review before multi-tenant production rollout.

### 16.5 Required repository validation

For each implementation phase, use the repository-required Nix development shell when available and run:

```text
cargo check --quiet --message-format=short
cargo test --quiet --message-format=short
cargo fmt --check
```

Run targeted tests during iteration, then workspace checks at the phase boundary.

## 17. Concrete file-level change map

### Workspace and new crates

- `Cargo.toml`: workspace dependencies for cryptography, secret wrappers, constant-time comparison, and any cache/rate-limit support after dependency review.
- `crates/authz/Cargo.toml`, `crates/authz/src/{lib,ids,domain,model,tuple,condition,request,decision,evaluator,store,cache,error}.rs`: pure domain-qualified authorization core.
- `crates/control_plane/Cargo.toml`, `crates/control_plane/src/{lib,model,store,service,account,credential,token,session,tenant,scope,database,database_name,binding,audit,crypto,error}.rs`: global identity/control and binding-saga services.
- `crates/control_plane/src/schema/{mod,bundle,migrations}.rs`: `semantic.control` package, indexes, and migrations.
- `crates/control_plane/src/store/{semantic_db,memory}.rs`: production adapter and test store; add backend-specific adapters only if generic DB guarantees are insufficient.
- `crates/app/src/local_auth/{mod,model,store,service,schema,migrations}.rs` initially (extract to `crates/local_auth` if dependency boundaries justify it): protected `semantic.local_auth` repositories, local credential/token/session lifecycle, external shadow subjects, and database-domain authz store adapter.

### Existing data/database layer

- `crates/data/src/bundles/auth/mod.rs`: deprecate the ambiguous initial user schema or turn it into shared neutral field definitions; explicit global and local packages own persistence and migrations.
- `crates/db_core/src/backend.rs`, `transaction.rs`, `managed_schema.rs`: only the narrow capability/atomicity/revision changes established by Phase 1; add conformance traits/tests without coupling tenant auth to all DB operations.
- `crates/db_redb` and `crates/db_postgres`: root/local-auth transaction and uniqueness adapters or required backend guarantees; optional later PostgreSQL RLS support.

### App

- `crates/app/src/auth.rs`: move/re-export qualified principal/domain types, add authenticated and composite decision context; preserve `system` only for internal construction and add a private auth-less-main capability.
- `crates/app/src/session.rs`: split persisted `AuthSession` references from ephemeral `ConnectionSession` selection.
- `crates/app/src/scope.rs`: replace in-memory principal-keyed scope ownership with registry-backed UUID/name-resolving `DatabaseManager` and leases; compatibility façade during rename.
- `crates/app/src/context.rs`: private mode-specific construction, selector canonicalization, auth scope/root/binding/local resolution, composite authorization methods, `AuthorizedDb`/object-store access.
- `crates/app/src/db.rs`: database descriptor/provider integration and authorized façade; do not pass network principals as provider authorization.
- `crates/app/src/command.rs`: command policy metadata/adapter, new scope/database/account/admin commands, deprecate raw open, authorize every built-in.
- `crates/app/src/file.rs`, `object_store.rs`, `media.rs`: file-object permissions, composite cache/job keys, signed capability integration.
- `crates/app/src/config.rs`, `error.rs`, `lib.rs`: explicit auth-less/multi-tenant enum, main DB identity, injected control plane/local auth/authz engines, typed settings/errors, tests.

### Server

- `crates/server/src/auth.rs`: global/local bearer/cookie routing and authenticators, local login selector resolution, remove external-system mapping, keep header resolver test-only.
- `crates/server/src/router.rs`: authentication/request-ID middleware, auth endpoints, selection resolution, status/`WWW-Authenticate` mapping.
- `crates/server/src/ws.rs`: upgrade authentication/origin, random connection IDs, ordered selection, revocation, limits.
- `crates/server/src/file.rs`: shared authorized context, CSRF for mutations, file capabilities, safe errors.
- `crates/server/src/config.rs`, `main.rs`, `lib.rs`, `error.rs`: mode-isolated route trees, root-free auth-less startup/main identity, multi-tenant root startup/migrations/bootstrap/admin/adoption CLI, readiness, graceful task shutdown.

### RPC, UI, docs, and tests

- `crates/rpc/src/protocol.rs`: version/capabilities, `DatabaseSelector`, UUID+name database references, qualified identity responses, and optional request selection context.
- `crates/rpc/src/client.rs`, `transport/http_client*.rs`, `transport/ws_client*.rs`, `file.rs`: redacting credential providers, cookies/CSRF/upgrade headers, typed selections.
- `crates/ui/src/app.rs`, `main.rs`, new global/local auth/account/admin views, and `crates/ui_core/src/context/scope.rs`: identity-domain-aware login state plus separate auth-scope/database UUID/name context and selectors.
- `docs/ARCHITECTURE.md`: control-plane/data-plane and permission boundaries.
- New operator/security docs: bootstrap, migration, token/session policy, tenant provisioning/deletion, backup/restore, audit, key rotation, break-glass, incident response, and threat model.
- Integration tests in the owning crates plus shared multi-tenant fixtures in `crates/db_test` or a new narrowly scoped test-support module.

## 18. Rollout, rollback, and data migration

1. Ship qualified IDs/selectors and protected main UUID/name metadata with existing auth-less behavior intact; verify auth-less starts with no root dependency and only one fixed DB.
2. Ship root DB/registry/name uniqueness and local-auth packages dark. Dry-run adoption, then register the existing main UUID/name and verify descriptor/open/blob behavior without concurrent auth-less serving.
3. Bootstrap global admin, provision its local shadow/role through the binding saga, and enable global/local auth for canary users; keep DB-wide local permissions only.
4. Shadow root, local, and composite authz decisions against coarse grants. Alert on differences and pending/stale bindings without enforcing.
5. Enforce by transport/path in order: admin/auth APIs, HTTP RPC, files, WebSocket, background jobs. Test UUID and name selectors at each step.
6. Canary tenants, then cohorts; watch denials by gate/domain, auth latency, saga reconciliation, DB opens/pools, cache invalidation, session revocation, and root/local audit health.
7. Enable platform data elevation/break-glass only after MFA, audit, alerting, and review workflows pass. Add object-level permission only per query/operation capability; unsupported query shapes remain DB-wide-gated.

Rollback keeps root/local state and registrations but switches a canary back to the previous coarse composite evaluator. Returning an adopted database to `single_user_authless` requires the stopped-server assessment in Section 14.2; it must never turn authentication off for an externally reachable multi-tenant deployment. Schema migrations are forward-only; destructive cleanup waits until the compatibility window and a verified backup. Authorization model activation is separately reversible by atomically restoring the prior immutable root/local model versions.

## 19. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Building accounts, tokens, DB lifecycle, and full ReBAC/ABAC together is too large | Phase coarse database authorization first behind the final engine/API boundary; make object query enforcement a separate gate |
| Generic semantic DB lacks root/local-auth transactional guarantees | Capability contract and backend conformance before production; backend-specific adapters rather than unsafe emulation |
| Terminology collision with current `DbScopeId` | Introduce strong `AuthScopeId`/`DatabaseId` now; versioned compatibility aliases |
| Global and local accounts are accidentally conflated | Domain-qualified IDs and API types; never merge by username/email; adversarial same-identifier tests |
| Root and local allows accidentally union | One composite decision constructor with explicit AND semantics; private raw decisions/capabilities; truth-table tests |
| Cross-database binding updates are non-atomic | Pending/active saga, generations, durable outbox/reconciler, fail-closed access, outer-first revocation |
| Database names become authority or collide after normalization/rename | UUID-only persisted authority, one normalization routine, atomic global index, UUID-looking-name rejection, tombstones/aliases |
| Local auth data is exposed through ordinary database queries/backups/restores | Protected package and typed repositories; explicit export policy; quarantine credentials on UUID-changing restore |
| Auth-less convenience becomes an unauthenticated multi-DB server | Process-wide mode enum, one fixed main capability, loopback default, no registry/auth/admin routes, incompatible-config rejection |
| Admin “all DBs” becomes an unreviewed bypass | Model as explicit policy; no `system`; recent auth, reason, audit, rate limits, narrower roles |
| Dynamic policies become arbitrary code or non-deterministic | Typed condition AST, trusted attribute providers, bounded evaluation, immutable versions, no network/plugin callbacks in checks |
| Relation graph checks become slow or cyclic | Validation, budgets, visited set, batch APIs, revisioned caches, pathological benchmarks |
| Revocation races with cached/live sessions | Security versions, revisioned invalidation, socket close broadcast, current-read option for sensitive actions |
| Object authorization is bypassed by raw queries | Database-wide gating until planner-level mandatory constraints cover a query shape |
| Per-database handles/pools exhaust process or Postgres | Singleflight sharing, leases, LRU, global/per-tenant budgets, fair admission, saturation metrics |
| Root DB is a single high-value dependency | Least privilege, encryption/secret management, backups, restore drills, audit, multi-instance strategy, fail-closed readiness |
| Control DB compromise exposes token/password verifiers | High-entropy opaque tokens, external pepper, Argon2id, key rotation, limited retention, no plaintext secrets |
| Cross-instance state diverges | Durable root source of truth, revision/checkpoint protocol, invalidation channel, bounded cache TTL, conformance tests |

## 20. Open decisions to resolve before implementation

1. Which backends are supported for the root DB in the first production release: redb only, PostgreSQL only, or both? Recommendation: support redb for single-node/local and PostgreSQL for multi-instance, but do not claim multi-instance until invalidation and transactional conformance pass.
2. Should a later release support Unicode names or privileged tombstone release? V1 deliberately uses the specified lowercase-ASCII normalization, 30-day old-name alias, and permanent tombstones. Any relaxation requires a versioned migration/ADR and cannot alter UUID-based authority.
3. Does global login identity uniqueness apply globally or per tenant? Recommendation: globally unique normalized identities for global accounts, tenant memberships separately. Local login uniqueness is independently enforced per database and never merged with global identity; support external issuer+subject identities.
4. May a global auth scope span tenants? Recommendation: no by default; allow only an explicit platform scope/grant with admin audit. Local principals never enter global auth scopes.
5. What exactly do platform roles cover, and is local-policy bypass ever allowed? Recommendation: global management roles do not imply reads; `platform_data_admin` uses the specified short-lived root-attested elevation evaluated by the protected local base rule, and separately enabled break-glass is the only local-gate bypass.
6. Does every database enable local accounts, or may a database use only global bindings? Recommendation: install the protected schema everywhere for consistent policy, but allow local credential issuance to be disabled per database.
7. Which local token/session routing representation and verifier-pepper isolation is required? Recommendation: visible UUID routing hint plus opaque secret, stored UUID audience, and per-database/derived pepper keys; never a mutable name or database scan.
8. Which browser authentication UX is required first: server session cookie, access/refresh token pair, or external OIDC? Recommendation: server session cookie first, opaque PATs for API/native, with identity-domain-aware routing.
9. What is the revocation consistency SLO across instances and live sockets for root grants, bindings, and local policy? This determines invalidation transport and cache TTL.
10. What atomic/conditional guarantees can the current semantic redb and PostgreSQL layers expose without destabilizing core behavior, including local auth and name uniqueness?
11. Where are provider/database credentials and root/local pepper keys stored and rotated? Root DB should contain references/encrypted envelopes, not plaintext credentials returned through APIs.
12. Which hierarchy relations and dynamic attributes are product requirements for v1, and which are deferred? Freeze a small typed vocabulary before building a general policy editor.
13. Are explicit denies required? Recommendation: defer; default deny plus allow relations first. Root-boundary denial always dominates local allow regardless.
14. Which query forms must support object-level permissions? Define a capability matrix before Phase 6; unrestricted SQL cannot coexist with per-object secrecy without planner/backend enforcement.
15. How are local-auth records exported/restored, especially when storage is cloned under another UUID? Recommendation: preserve UUID only for disaster recovery of the same registered DB; otherwise quarantine credentials/bindings and require explicit rebind.
16. What audit retention, privacy, export, dual-write, and fail-closed requirements apply to root vs local audit per deployment?
17. What tenant offboarding/erasure retention applies to names/tombstones, databases, blobs, root metadata, local identities, backups, tokens, and audit records?
18. Should auth-less non-loopback binding be forbidden or available behind an explicit unsafe flag? Recommendation: permit only a conspicuous opt-in for trusted environments, never as a multi-tenant substitute.

## 21. Definition of done

The multi-tenant system is complete only when:

- multi-tenant mode cannot start or serve tenant data without a healthy migrated root DB and production authenticator;
- auth-less mode starts and serves its persisted UUID/name `main` database without a root DB, opens no other DB, exposes no auth/registry/admin switching surface, derives no principal from the network, binds safely by default, and never becomes a fallback for multi-tenant failure;
- no external path can become `system` or select/query the root DB;
- global and per-database local accounts, credentials, tokens, sessions, groups, external/shadow bindings, tenants, auth scopes, databases, grants, and root/local policies survive restart and support domain-scoped audited lifecycle operations;
- every database has an immutable UUID and globally unique normalized name; UUID/name inputs canonicalize identically, while tokens, sessions, grants, bindings, permissions, caches, jobs, and audit use UUID; rename/reuse/alias behavior is unambiguous and tested;
- local principals and credentials are permanently bound to one database and cannot assert global identity; global principals require an active root grant/binding and local permission;
- effective authorization is root boundary AND local policy where both apply, never an unsafe union, and revocation/reconciliation fail closed;
- arbitrary selectors never grant authority, and every HTTP, WebSocket, RPC, file, and background path uses the same mode-appropriate authenticated/authorized context;
- platform administration and tenant data access are separate; elevation/break-glass is database-bound, least-privilege-capable, revocable, recently strongly authenticated where required, and durably audited/alerted;
- database handle/pool lifecycle is shared, bounded, race-safe, and tenant-fair;
- `semantic_authz` is independently testable and supports versioned relation models, scoped/dynamic subjects, hierarchy, consistency revisions, bounded evaluation, and fail-closed decisions;
- object-level authorization is not advertised for any query shape that can bypass mandatory constraints;
- the cross-domain/two-tenant selector matrix, auth-less mode matrix, backend conformance, binding/revocation races, security tests, load isolation, backup/restore/rebind drill, adoption, and rollback rehearsal pass;
- operator, auth-less/upgrade, migration, name lifecycle, local identity, threat-model, recovery, audit, break-glass, and key-rotation documentation is complete.
