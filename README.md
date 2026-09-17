# diffpack-engine

The Rust engine behind [diffpack.io](https://www.diffpack.io): it fetches a package archive from
crates.io, npm, PyPI or RubyGems, extracts it in memory, diffs two versions, and hands the browser
a tree it can render.

Published to npm as **[`@philfreshman/diff-wasm`](https://www.npmjs.com/package/@philfreshman/diff-wasm)**,
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
3. `git tag v0.1.1 && git push origin v0.1.1`.

`release.yml` refuses a tag that disagrees with `Cargo.toml`, builds, and publishes with npm
provenance. Then bump `@philfreshman/diff-wasm` in diffpack — or let Renovate open that PR, where
the full end-to-end suite runs against the new module.
