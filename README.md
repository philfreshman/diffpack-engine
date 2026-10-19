# diffpack-engine

Fetches, extracts and diffs package archives from npm, crates.io, PyPI and the Go module proxy, as a WebAssembly module. It is the engine behind [diffpack.io](https://www.diffpack.io).

## Install

```bash
npm install @philfreshman/diffpack-engine
```

## Usage

Runs in a browser page or a Web Worker.

```js
import init, {
  build_diff_tree_for_package,
  get_diff_for_comparison,
} from "@philfreshman/diffpack-engine";

await init();

const tree = await build_diff_tree_for_package("npm", "left-pad", "1.2.0", "1.3.0", 0.75, false);
const { data, isDiff } = get_diff_for_comparison("npm", "left-pad", "1.2.0", "1.3.0", "index.js", undefined, false);
```

Registries are `npm`, `crates`, `pypi` and `go`. `0.75` is the similarity threshold for rename detection, and the last argument ignores whitespace when `true`. `get_diff_for_comparison` reads from the session cache, so fetch both versions first with `build_diff_tree_for_package` or `prefetch_package`.

## Errors

A failed download rejects with an `Error`. Its `message` is a sentence for people, such as `Couldn't download https://static.crates.io/crates/itoa/itoa-9.9.9.crate (HTTP 404)`, and it carries these properties:

| Property | Value |
| :- | :- |
| `code` | Why the download failed, as listed below. Always present. |
| `status` | The HTTP status, when there was an answer. |
| `resetsAt` | An ISO 8601 time, when the diffpack proxy said when it comes back (`quota_exhausted`). |

| `code` | When |
| :- | :- |
| `not_found` | The registry answered 404. |
| `rate_limited` | The registry answered 429. |
| `upstream_failed` | The registry answered with any other status that is not a success, or the diffpack proxy sent an answer whose body could not be read. |
| `network` | `fetch` itself failed: offline, blocked by CORS, or the host could not be found. |
| any other | An answer from the diffpack proxy (it sends a `Diffpack-Proxy-Error` header) passes its own `error` code through, for example `quota_exhausted` or `archive_not_listed`. |

Other errors, such as an unsupported registry or an archive that cannot be extracted, reject with a plain string and have no `code`. These error values are part of the wasm contract, so changing a code or its meaning is a breaking change.

## Build and test

Needs Rust with the `wasm32-unknown-unknown` target and [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/).

```bash
cargo test
wasm-pack test --headless --chrome
wasm-pack build --release --target web --scope philfreshman
```
