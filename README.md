# diffpack-engine

The Rust engine behind [diffpack.io](https://www.diffpack.io): it fetches a package archive from
crates.io, npm, PyPI or the Go module proxy, extracts it in memory, diffs two versions, and hands
the browser a tree it can render.

Published to npm as **[`@philfreshman/diffpack-engine`](https://www.npmjs.com/package/@philfreshman/diffpack-engine)**,
a `wasm-pack --target web` module. The app consumes that published version; it does not build this
crate. This repo was split out of [`philfreshman/diffpack`](https://github.com/philfreshman/diffpack)
with its history intact.

## What it exports

Three `#[wasm_bindgen]` entry points, all driven from the app's diff worker:

| Function | Does |
| --- | --- |
| `prefetch_package(registry, pkg, version)` | Fetch and extract one version into the session cache |
| `build_diff_tree_for_package(registry, pkg, from, to, similarity_threshold, ignore_whitespace)` | Diff two versions, return the file tree with statuses, counts and detected renames |
| `get_diff_for_path(filename, old_path, ignore_whitespace)` | The unified diff for one path in the active comparison |

Extraction is cached per `registry:package:version` for the session and shared by `Rc`, so diffing
A→B then B→C fetches B once. `build_diff_tree_for_package` is the call that establishes which pair
`get_diff_for_path` reads.

### The Rust surface

The crate is also a plain Cargo dependency. [`diffpack-server`](https://github.com/philfreshman/diffpack-server)
computes diffs with it natively — no wasm, no browser — and the items below are supported API for
that consumer: a change to any of their signatures, or to `get_diff_content`'s output, is a breaking
change rather than an internal one.

| Item | Does |
| --- | --- |
| `get_diff_content(filename, from, to, ignore_whitespace)` | The unified diff for one file. Its **output** is the contract, not just its signature — see below |
| `whitespace_mode(ignore_whitespace)` → `WhitespaceMode` | The `Exact`/`IgnoreAll` choice, for a caller configuring its own `TextDiff` |
| `build_tarball_url(registry, pkg, version)` | The archive URL for npm (scoped names included) and crates.io |
| `select_pypi_sdist_url(&[PyPiUrl])` | The sdist-then-wheel preference order, over a parsed `PyPiResponse` |
| `escape_go_module_path`, `build_go_zip_url`, `strip_go_module_root` | The Go module proxy's path escaping, its zip URL, and the `<module>@<version>/` prefix every entry carries |
| `extract_archive_bytes(bytes)`, `build_diff_tree(..)` | Extraction and the tree builder, which `examples/bench.rs` also drives |

`get_diff_content` renders a `--- from/{f}` / `+++ to/{f}` header, then one line per change as sign
(`-`, `+` or a space), a space, and the line. The tree's counts are taken from the same lines, so a
second implementation that renders them differently makes the tree and the file view disagree about
the same file. That is the reason these are exported rather than rewritten on the far side.

`WhitespaceMode` is `similar`'s, re-exported here so a dependent takes the type from this crate
rather than from a `similar` of its own that might resolve to a different version.

`DiffTreeBuilder` and everything under extraction stay private; `build_diff_tree` is the door.

## The sibling repositories

Three repositories carry diffpack, and they are meant to be checked out side by side under one
parent directory — the layout the app's `DIFFPACK_ENGINE_LOCAL=../diffpack-engine/pkg` assumes:

| Repo | Sibling path | Remote | What it is |
| --- | --- | --- | --- |
| diffpack | `../diffpack` | `philfreshman/diffpack` | The web app at [diffpack.io](https://www.diffpack.io) — TanStack Start, the UI, the registry search, the worker that calls the three functions above. It consumes the published npm package and never builds this crate. |
| diffpack-engine | *this checkout* | `philfreshman/diffpack-engine` | This crate. |
| diffpack-server | `../diffpack-server` | `philfreshman/diffpack-server` | An MCP server that computes diffs with this crate as a **native** Cargo dependency, pinned by git tag. It consumes the Rust surface above; nothing here or in the app depends on it. |

To try a change in the app before releasing it, build here and point the app at `pkg/`:

```bash
wasm-pack build --release --target web --scope philfreshman
cd ../diffpack && DIFFPACK_ENGINE_LOCAL=../diffpack-engine/pkg bun run dev
```

Anything short of that — a released version, or nothing at all — means the app is running the
pinned `@philfreshman/diffpack-engine` from npm, not what is in this working tree.

## Working on it

Needs a Rust toolchain with the `wasm32-unknown-unknown` target, and
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/).

```bash
cargo test                                              # the host-side suite
cargo fmt --all                                         # what CI checks
wasm-pack build --release --target web --scope philfreshman
```

The build writes `pkg/`, which is gitignored: it is generated on every build and published from CI,
never committed.

### Tests

`cargo test` compiles for the host and covers everything the module is built out of — extraction,
path normalisation, the registry URL builders, rename detection, the tree's statuses and counts, the
serialised shape TypeScript reads, and `examples/bench.rs`'s own tests. It is the fast suite and the
one to reach for.

`tests/public_api.rs` is the Rust surface above, tested as a dependent sees it. An integration test
is its own crate, so it links `diffpack-engine` exactly the way diffpack-server does and can reach
nothing `lib.rs` has not exported: an item that stops being public fails the build there rather than
at the far end of a version bump.

It cannot reach the `#[wasm_bindgen]` functions themselves: they need a `fetch` and a `JsValue`, and
constructing a `JsValue` off `wasm32` aborts the process. Those are `tests/web.rs`, in a real
browser, against real crates.io archives:

```bash
wasm-pack test --headless --chrome
```

That suite matters more here than it did in the monorepo. There, a signature change was caught
incidentally by diffpack's Playwright run on the same PR. Now the app consumes a pinned version, so
this is the last thing between a broken binding and a release.

### Benchmarking

`examples/bench.rs` runs extraction and the tree builder natively, without a browser or a fetch in
front of them — which is how the release profile in `Cargo.toml` was chosen. Its comment records the
measurement: `opt-level = 's'` with `lto = true` is a 587 KB module that halved compute against the
515 KB `opt-level = 'z'` build.

```bash
cargo run --release --example bench -- --help
```

## Releasing

The version in `Cargo.toml` is the single source of truth — wasm-pack copies it into the generated
`pkg/package.json`, which is what npm publishes. A release is:

1. Bump `version` in `Cargo.toml`, and `cargo check` so `Cargo.lock` follows.
2. Merge to `main`.
3. `git tag v0.3.0 && git push origin v0.3.0`.

`release.yml` refuses a tag that disagrees with `Cargo.toml`, builds, and publishes with npm
provenance. Then bump `@philfreshman/diffpack-engine` in diffpack — or let Renovate open that
PR, where the full end-to-end suite runs against the new module.

The tag is also how diffpack-server pins this crate, which npm never sees. A release that changes
the Rust surface is one diffpack-server has to move its `tag = "v..."` to; one that does not, it can
ignore.
