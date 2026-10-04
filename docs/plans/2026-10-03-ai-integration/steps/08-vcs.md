# Step 08: Workspace and VCS (`semantic_agent_vcs`)

Wave W2. Depends on: 01 (only for the diff model). Agent: opus. Runs in parallel with
steps 02 and 06.

Read: research/t3code-orchestration.md §12 (checkpointing: mechanism, ordinal scheme,
capture, rollback), §13 (worktrees, cleanup, workspace leases), §21.3 (Rust shape),
research/agent-protocols.md §7.4.13 (path validation).

## Goal

A DB-free, reusable crate for everything the orchestrator does with workspace
directories:

1. path containment (safe resolution of user and agent supplied paths);
2. git repository detection and status;
3. **checkpoints**: snapshots of the working tree without touching the user's index
   or HEAD;
4. diffs between checkpoints, and from a checkpoint to the working tree;
5. restore to a checkpoint;
6. worktree create, list and remove;
7. per-repository serialization (lease).

Implementation shells out to `git` via `tokio::process` (consistent with t3code;
avoids heavy deps). It requires git ≥ 2.30. Detect the version once per process and
error clearly if it is older.

## API

```rust
pub struct WorkspacePath { root: PathBuf }                       // canonicalized root
impl WorkspacePath {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, VcsError>;              // must exist, canonicalize (symlinks resolved)
    pub fn resolve(&self, rel_or_abs: &str) -> Result<PathBuf, VcsError>;      // rejects escape (.., symlink out), fails closed
    pub fn contains(&self, path: &Path) -> bool;
}
pub fn is_within_roots(path: &Path, roots: &[PathBuf]) -> bool;               // allow-list check used by the orchestrator (K8)

pub struct Git { /* binary path, version */ }
impl Git {
    pub async fn detect() -> Result<Self, VcsError>;
    pub async fn open(&self, path: &Path) -> Result<Option<Repo>, VcsError>;   // None if not inside a repo
}
pub struct Repo { pub root: PathBuf, pub common_dir: PathBuf, pub is_worktree: bool }
impl Repo {
    pub async fn status(&self) -> Result<RepoStatus, VcsError>;                // branch, head, dirty file count, ahead/behind if upstream
    pub async fn capture(&self, ns: &CheckpointNamespace, ordinal: u64) -> Result<CheckpointRef, VcsError>;
    pub async fn capture_if_missing(&self, ns: &CheckpointNamespace, ordinal: u64) -> Result<CheckpointRef, VcsError>;
    pub async fn diff(&self, from: DiffSide, to: DiffSide, opts: DiffOptions) -> Result<DiffResult, VcsError>;
    pub async fn restore(&self, cp: &CheckpointRef) -> Result<(), VcsError>;
    pub async fn delete_checkpoints_after(&self, ns: &CheckpointNamespace, ordinal: u64) -> Result<u32, VcsError>;
    pub async fn worktree_add(&self, req: WorktreeRequest) -> Result<WorktreeInfo, VcsError>;   // {path, branch, base_ref, start_from_remote}
    pub async fn worktree_remove(&self, path: &Path, force: bool) -> Result<(), VcsError>;
    pub async fn worktrees(&self) -> Result<Vec<WorktreeInfo>, VcsError>;
    pub async fn branches(&self, query: Option<&str>, limit: usize) -> Result<Vec<BranchInfo>, VcsError>;
}
pub struct CheckpointNamespace(String);   // e.g. thread id; sanitized + hashed into the ref path
pub struct CheckpointRef { pub ref_name: String, pub commit: String, pub ordinal: u64 }
pub enum DiffSide { Checkpoint(CheckpointRef), WorkingTree, Head }
pub struct DiffOptions { pub max_bytes: usize /* default 2 MiB */, pub ignore_whitespace: bool, pub paths: Vec<String> }
pub struct DiffResult { pub patch: String, pub truncated: bool, pub files: Vec<FileStat> /* numstat: path, old_path, additions, deletions, binary */ }
pub struct RepoLease;  // acquired via LeaseRegistry::acquire(repo_common_dir).await, serializes capture/restore/worktree ops per repository
```

## Checkpoint mechanism (normative, from research §12.1)

1. Create a temp index file under `common_dir` (`semantic-checkpoint-index-<uuid>`)
   and set `GIT_INDEX_FILE` for all following commands.
2. `git read-tree HEAD`. For an unborn HEAD, use an empty tree.
3. `git add -A -- .`. On failure caused by nested repos without commits, retry
   excluding them and record the exclusions.
4. `git write-tree`, then
   `git commit-tree <tree> [-p HEAD] -m "semantic checkpoint <ns>/<ordinal>"` with
   author and committer env `Semantic Agent <agents@semantic.invalid>` and a fixed
   timezone.
5. `git update-ref refs/semantic/agents/checkpoints/<hash(ns)>/<ordinal> <commit>`
   with `-c core.fsync=objects,reference`.
6. Remove the temp index (also on error, via a guard).

The user's index, HEAD and working tree are never modified by `capture`. Ignored
files are not captured (document this). `restore`:

1. `git restore --source <commit> --worktree --staged -- .` (when the tree is non-empty);
2. `git clean -fd -- .`;
3. `git reset --quiet -- .`.

`restore` is only allowed when the caller asserts the workspace is isolated. The
orchestrator decides this (step 10); this crate documents the hazard.

## Diff

`git diff --patch --find-renames -z`/`--numstat -z` between `<from>^{commit}` and
`<to>^{commit}`, or `<from>` and the working tree (via a temp checkpoint of the
working tree when untracked files must be included; document the choice). Output is
capped at `max_bytes`, with the truncated flag set. The `patch` must be parseable by
`semantic_agent::diff::parse_unified_diff`; test this.

## Worktrees

`worktree_add` behaviour:

* creates `<repo_parent>/.semantic-worktrees/<repo_name>/<slug>`, or an explicit
  path, which must be absolute and not exist;
* branch is new (`-b`) from `base_ref`;
* optional `git fetch` first when `start_from_remote`;
* progress callback optional (later).

`worktree_remove` refuses dirty worktrees unless `force`.

## Tests (temp git repos via `tempfile`; set `GIT_CONFIG_GLOBAL=/dev/null`, `user.name/email` env for determinism)

* Path containment: `..`, absolute outside, symlink escaping, symlink inside allowed,
  non-existent paths (resolve parent).
* Capture on a clean repo, a dirty tracked file, an untracked file, deleted files and
  an unborn HEAD. The user's index is unchanged (compare `git diff --cached` before
  and after). HEAD is unchanged.
* `capture_if_missing` is idempotent.
* Diff between two checkpoints: added, modified, deleted and renamed files, binary
  file. The result parses with the core diff parser and its numstat matches.
* Restore to an earlier checkpoint restores content and removes newly created files.
* Worktree add, list and remove; a dirty remove is refused.
* Concurrent captures on the same repo are serialized by the lease (spawn 5, all
  succeed, distinct ordinals).
* Non-git directory: `open` returns `None`.

## Acceptance

* Tests green (they require `git` in the devshell; check `flake.nix` and add git to
  the devshell if missing, noting it in the commit).
* One commit: "Add agent workspace VCS crate: checkpoints, diffs, worktrees".
