# Review 2 follow-up

The reviewer inputs are unchanged. This round addresses the one remaining
functional finding without changing the database or plugin protocol.

## Resolution

- **1: describe recovery.** Every invocation error now uses the existing
  five-second retry backoff, including plugin-defined codes such as
  `network_error`, `http_error` and `auth_failed`. The code allowlist was removed.
  Output shape and descriptor decoding errors remain cached for the generation.
  Successful descriptors still reuse the existing catalog validation cache.
- **2: optional nits.** LeafKey and SourceRef debug output remain unchanged;
  neither causes a functional problem. The small optional-field DTO test remains
  as coverage for the shared command contract introduced in review 1.

## Validation

All 33 SDK tests with the testing feature passed, including retry suppression
at four seconds and recovery at five seconds for unknown `network_error` and
plugin-defined `invalid_schema` codes. Generation replacement still recovers a
cached successful descriptor whose schema is invalid. Independent inspection
confirmed the host output shape/arity/decoding failure path is unchanged.

Final workspace short-format check, `cargo fmt --all` and whitespace check
passed. Cargo commands used the Nix devshell and the main clone's shared target
directory. No broad stored oracle rerun or new commit was made.
