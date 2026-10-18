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

## Build and test

Needs Rust with the `wasm32-unknown-unknown` target and [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/).

```bash
cargo test
wasm-pack test --headless --chrome
wasm-pack build --release --target web --scope philfreshman
```
