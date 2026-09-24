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
that consumer: a change to any of their signatures, or to `get_diff_content`'s or `build_patch`'s
output, is a breaking change rather than an internal one.

| Item | Does |
| --- | --- |
| `get_diff_content(filename, from, to, ignore_whitespace)` | The unified diff for one file. Its **output** is the contract, not just its signature — see below |
| `build_patch(filename, from, to, ignore_whitespace)` → `Patch` | One file's view from its content in each version (`None` where a version has no file there): a sentence when neither has it, every line against `/dev/null` when one side does, the file itself when the two are byte-identical, `get_diff_content` otherwise. `get_diff_for_path` renders through it. Its output is the contract too |
| `whitespace_mode(ignore_whitespace)` → `WhitespaceMode` | The `Exact`/`IgnoreAll` choice, for a caller configuring its own `TextDiff` |
| `build_tarball_url(registry, pkg, version)` | The archive URL for npm (scoped names included) and crates.io |
| `select_pypi_sdist_url(&[PyPiUrl])` | The sdist-then-wheel preference order, over a parsed `PyPiResponse` |
| `escape_go_module_path`, `build_go_zip_url`, `strip_go_module_root` | The Go module proxy's path escaping, its zip URL, and the `<module>@<version>/` prefix every entry carries |
| `extract_archive_bytes(bytes)`, `build_diff_tree(..)` | Extraction and the tree builder, which `examples/bench.rs` also drives |

`get_diff_content` renders a `--- from/{f}` / `+++ to/{f}` header, then one line per change as sign
(`-`, `+` or a space), a space, and the line. The tree's counts are taken from the same lines, so a
second implementation that renders them differently makes the tree and the file view disagree about
the same file. That is the reason these are exported rather than rewritten on the far side.

`build_patch` is the whole file view around that diff, and `Patch { data, is_diff }` is what it
returns. `is_diff` is `Patch`'s serialised name as well — the shape diffpack-server stores — while
`get_diff_for_path` renames it to `isDiff` for the browser. An added or removed file is split on
`\n`, so one ending in a newline gets an empty last `+`/`-` line that the tree does not count; that
is current behaviour, kept as it is until it can be changed in this one place.

`WhitespaceMode` is `similar`'s, re-exported here so a dependent takes the type from this crate
rather than from a `similar` of its own that might resolve to a different version.

`DiffTreeBuilder` and everything under extraction stay private; `build_diff_tree` is the door.

The tree it returns has one node per path, with one exception to allow for: a path that is a file
in one version and a directory in the other — `lib` a module in one and a folder holding
`lib/index.js` in the other — or both in one, which a malformed archive can manage, is two sibling
nodes with the same `path`, told apart by `type`, the old version's first. A file node never has
anything under it. Each of the two still follows the usual rules, so either can be the only node at
that path: a file moved into the folder is listed as the rename beneath it, not at its old path,
and a folder left with nothing in it is not listed.

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
.githooks/pre-commit                                    # the four gates CI runs
wasm-pack build --release --target web --scope philfreshman
```

The build writes `pkg/`, which is gitignored: it is generated on every build and published from CI,
never committed.

### Checks

Four gates, run by `.githooks/pre-commit` before every commit and again by `ci.yml` on every PR.
Same commands in both places, so a commit that passes locally fails CI only if the RustSec advisory
database moved in between.

```bash
cargo fmt --all --check                                   # formatting
cargo clippy --all-targets --all-features -- -D warnings  # lints
cargo deny --all-features check                           # licences, advisories, sources
cargo audit --deny warnings --no-yanked                   # advisories, unmaintained, unsound
```

The hook is per-clone and off until you turn it on:

```bash
rustup component add clippy
cargo install --locked cargo-deny cargo-audit
git config core.hooksPath .githooks
```

It takes about two seconds on a warm cache. `git commit --no-verify`, or `SKIP_PRECOMMIT=1`, skips
it; CI does not.

`deny.toml` is the supply-chain policy: permissive licences only, no git dependencies, no registry
but crates.io, yanked crates denied. It matters more than it would in an application — this crate
ships as a wasm module inside somebody else's bundle, and Renovate automerges dependency PRs, so
these two jobs are what an automerge has to get past.

`cargo audit` overlaps `cargo deny check advisories`; it is kept because it is the one with
`--deny warnings`, which fails on unmaintained and unsound crates too. `--no-yanked` is not a
weakening — `deny.toml` denies yanked crates — it is there because cargo-audit's own yanked check
updates the git crates.io index, which measured 17 minutes against 0.7s without it.

### Dependencies

Crate updates are Renovate's job, not anyone's. `renovate.json` has it read `Cargo.toml` and
`Cargo.lock` twice a month, open one PR per crate, and let GitHub merge each one the moment the
suite above goes green — no review, no queue. A full `Cargo.lock` refresh runs on the 1st.

Cargo ranges here are minimums (`"1.0"`, `"0.4"`), so most of those PRs change `Cargo.lock` and
nothing else. That is the intended split: the manifest says what the crate needs, the lock says what
it was built and tested against. It does mean the diff is rarely readable on its own — the four
checks are what actually read it, which is why all seven CI jobs are required checks on
`development`. A PR whose checks cannot run is a PR that never merges.

Two things are deliberately not automerged:

- **wasm-pack and the wasm-bindgen family.** Both decide the generated JS glue and the module's
  bytes, and the two are version-locked against each other. `wasm-pack test --headless --chrome` can
  pass on a pair that still fails in a consumer's bundler, so these get looked at. The wasm-bindgen
  crates arrive as one grouped PR; `WASM_PACK_VERSION`, pinned in both workflows, is picked up by a
  custom manager so the pin cannot quietly rot.
- **Anything less than three days old** (`minimumReleaseAge`). This crate ends up inside other
  people's bundles, and three days is the window in which a compromised or broken release is
  normally yanked.

Every PR body carries a Diffpack link per crate — `currentVersion → newVersion`, pointing at
diffpack.io — so the actual contents of an update are one click from the PR. That is the same
`prBodyDefinitions` trick diffpack uses on its own npm dependencies.

Renovate itself is the Mend GitHub App, enabled per repository at
<https://github.com/apps/renovate>. Nothing in this repo turns it on; if no dependency PRs and no
Dependency Dashboard issue ever appear, the app has not been given access to it.

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
2. Merge the work to `development`, then open `development` → `main` and merge that.
3. `git tag v0.3.0 && git push origin v0.3.0`, on `main`.

`development` is the integration branch: feature branches target it, and `main`
only ever takes a `development` → `main` PR. `ci.yml` runs on PRs into either and on the
merge commit each ends up with, so the commit a tag is cut from has been
through the suite twice. `release.yml` is the only thing keyed to the tag.

`release.yml` refuses a tag that disagrees with `Cargo.toml`, builds, and publishes with npm
provenance. Then bump `@philfreshman/diffpack-engine` in diffpack — or let Renovate open that
PR, where the full end-to-end suite runs against the new module.

The tag is also how diffpack-server pins this crate, which npm never sees. A release that changes
the Rust surface is one diffpack-server has to move its `tag = "v..."` to; one that does not, it can
ignore.
