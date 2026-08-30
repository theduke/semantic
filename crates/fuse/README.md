# Semantic FUSE filesystem

`semantic_fuse` projects a semantic database scope into an artificial
filesystem using [Fuser](https://crates.io/crates/fuser). It is a library: the
`semantic` binary in `semantic_cli` exposes it through `semantic fuse`.

## Filesystem layout

A mount contains four views:

```text
entities/
  <id-hash-prefix>/
    <escaped-id>.json

tree/
  <directory-title>/
    <id>.json
    <file-id>_<filename>

files/
  <id-hash-prefix>/
    <escaped-id>/
      <filename>

blobs/
  <first-two-hash>/
    <sha256>
```

The extension of entity documents is `.json` or `.yaml`, according to the
configured format. `entities/` is the canonical metadata view. `tree/`
projects directory entities as directories, other entities as `<id>.ext`, and
file entities as `<id>_<filename>`. `files/` provides file content grouped by
file-entity ID, while `blobs/` provides content addressed by its SHA-256 hash.

Semantic directories form a graph rather than a strict tree. A directory or
entity with multiple parents therefore appears at every applicable path.
Directory titles are used directly; an ID-derived suffix is added only when
sibling titles collide. Names that cannot be represented safely as path
components are escaped. Entity and file prefix directories use a hash of the
entity ID so common ID prefixes do not create oversized directories.

The mount configuration selects one database scope and one entity document
format. To browse multiple independent scopes, create a separate mount for
each scope.

## Runtime and I/O

Fuser invokes synchronous filesystem callbacks. The filesystem retains the
calling Tokio runtime handle and uses `block_on` to wait for asynchronous
operations through the native, thread-safe `RpcClient` abstraction. Multiple
Fuser workers may serve different open files concurrently; network waits use
per-file-descriptor locks rather than holding the global filesystem-state lock.

Raw files use the kernel page cache and read-ahead. Each open file descriptor
also keeps an offset-to-EOF API stream for sequential reads. Small forward gaps
are drained from that stream, while distant or backward seeks use one exact
range request without discarding a useful sequential stream. A second
contiguous read at the new location promotes it to the active stream. A stream
that was closed by an idle server is resumed from its exact byte cursor with a
bounded retry count. The server passes ranges to the backing object store and
streams the response, so neither side loads a whole blob for every FUSE read.

The `semantic` CLI passes Ctrl-C and SIGTERM to the mount as an explicit
shutdown request. The FUSE session is unmounted and its worker is joined before
the command exits. A crash or SIGKILL can still leave a disconnected kernel
mount behind. On the next start, the library recognizes the mountpoint's
`ENOTCONN` error, lazily detaches that stale mount with `fusermount3` (or
`fusermount`), and mounts the new session. Healthy active mounts are never
detached by this recovery path.

New file uploads use a bounded channel to stream bytes directly to the upload
endpoint with backpressure; they are not accumulated in a local temporary file
by default. Writes must arrive at the current sequential offset. An
out-of-order write aborts the upload and returns an error. When the expected
size is available, the upload sends it as the optional content length so the
server can reject an incomplete body.

Entity JSON/YAML writes are buffered until commit because the complete
document must be parsed and validated as an entity. In the tree view, `.json`,
`.yaml`, and `.yml` files are first parsed as entity metadata. If parsing or
entity-ID extraction fails there, the buffered content falls back to the
regular file-entity upload flow. Other tree files use the direct streaming
upload flow from their first write.

## Mutations and limitations

The current implementation supports:

- creating or replacing entity documents;
- uploading new files below directories in `tree/`;
- moving existing tree entries between directories; and
- a read-only mount mode that disables mutations.

Current limitations:

- one scope is exposed per mount;
- the directory/entity snapshot is loaded when the mount starts;
- creating and removing directories with `mkdir`/`rmdir` is not supported;
- replacing the raw content of an existing file is not supported.
