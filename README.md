# diffpack-engine

Fetches, extracts and diffs package archives from npm, crates.io, PyPI, Packagist (Composer) and the Go module proxy, as a WebAssembly module. It is the engine behind [diffpack.io](https://www.diffpack.io).

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

Registries are `npm`, `crates`, `pypi`, `composer` and `go`. A Composer package is named `vendor/package`, and its version matches with or without a leading `v` (`3.12.1` or `v3.12.1`). `0.75` is the similarity threshold for rename detection, and the last argument ignores whitespace when `true`. `get_diff_for_comparison` reads from the session cache, so fetch both versions first with `build_diff_tree_for_package` or `prefetch_package`.

### Composer and the download proxy

Packagist and the archive hosts send no CORS header, so in the browser Composer packages are downloaded through [diffpack-proxy](https://github.com/philfreshman/diffpack-proxy). Call `set_proxy_origin` once, after `init()` and before any Composer call:

```js
import init, { set_proxy_origin } from "@philfreshman/diffpack-engine";

await init();
set_proxy_origin("https://diffpack-proxy.philipswiezak.workers.dev");
```

- The origin is `https://<host>` with an optional port, or `http://localhost:<port>` or `http://127.0.0.1:<port>` for a local `wrangler dev`. One trailing slash is removed; a path, query or user info is refused with a plain string error, and the origin set before stays.
- The listing is read from `{origin}/v1/composer/p2/{vendor}/{package}.json` and each archive from `{origin}/v1/composer/archive?package={vendor}/{package}&url={URL as Packagist lists it}`. Both versions of one comparison share a single listing request.
- Other registries are fetched directly and do not need it.

## Errors

A failed download, or a Composer version that cannot be downloaded, rejects with an `Error`. Its `message` is a sentence for people, such as `Couldn't download https://static.crates.io/crates/itoa/itoa-9.9.9.crate (HTTP 404)`, and it carries these properties:

| Property | Value |
| :- | :- |
| `code` | Why it failed, as listed below. Always present. |
| `status` | The HTTP status, when there was an answer. |
| `resetsAt` | An ISO 8601 time, when the diffpack proxy said when it comes back (`quota_exhausted`). |

| `code` | When |
| :- | :- |
| `not_found` | The registry answered 404. |
| `rate_limited` | The registry answered 429. |
| `upstream_failed` | The registry answered with any other status that is not a success, or the diffpack proxy sent an answer whose body could not be read. |
| `network` | `fetch` itself failed (offline, blocked by CORS, or the host could not be found), the global scope has no `fetch`, or the download broke off after the answer arrived, in which case `status` is set. |
| `proxy_unreachable` | `fetch` failed on a URL of the diffpack proxy. Cloudflare answers without CORS once the proxy's free daily limit is used up, so it may be over that limit. |
| `proxy_not_configured` | A Composer package was requested before `set_proxy_origin` was called. |
| `no_such_version` | Packagist lists no such version of the Composer package. |
| `no_archive` | Packagist lists the version but no archive to download. |
| `unsupported_archive_host` | The archive is not on GitHub, Bitbucket or gitlab.com, or its URL does not have the shape those hosts use. The message names the host. |
| any other | An answer from the diffpack proxy (it sends a `Diffpack-Proxy-Error` header) passes its own `error` code through, for example `quota_exhausted` or `archive_not_listed`. |

Other errors, such as an unsupported registry or an archive that cannot be extracted, reject with a plain string and have no `code`. These error values are part of the wasm contract, so changing a code or its meaning is a breaking change.

## Rust API

diffpack-server uses the crate natively, in four steps:

1. `archive_source(registry, name, version)` says where the archive is, or which listing to read first (`pypi` and `composer`).
2. `choose_archive(registry, version, listing)` picks the archive URL from that listing. It takes the version since 0.5.0, because a Packagist listing covers every version of a package; PyPI ignores it.
3. Fetch the archive.
4. `unpack_archive(registry, name, version, bytes)` extracts its files.

For Composer, `choose_archive` returns the URL to download directly: a GitHub archive comes from `codeload.github.com`, which skips the `api.github.com` redirect that counts against GitHub's anonymous rate limit, and Bitbucket and gitlab.com archives come from the URL Packagist lists. Failures are plain `String` messages, the same sentences the wasm `Error` carries.

## Build and test

Needs Rust with the `wasm32-unknown-unknown` target and [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/).

```bash
cargo test
wasm-pack test --headless --chrome
wasm-pack build --release --target web --scope philfreshman
```
