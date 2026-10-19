# CLAUDE.md

diffpack-engine fetches, extracts and diffs package archives (npm, crates.io, PyPI, Go). It ships twice: as a wasm module on npm (`@philfreshman/diffpack-engine`) for the diffpack web app, and as a Cargo git dependency, pinned by tag, for diffpack-server. Cross-repo context is in `../CLAUDE.md`.

## Commands

```bash
cargo test                                   # host-side suite, fast and offline
cargo test <name>                            # tests whose name contains <name>
cargo test --test public_api                 # the Rust surface as diffpack-server sees it
wasm-pack test --headless --chrome           # the wasm bindings in a real browser, against crates.io
wasm-pack build --release --target web --scope philfreshman   # writes pkg/ (gitignored, never commit it)
.githooks/pre-commit                         # fmt, clippy -D warnings, cargo deny, cargo audit: what CI runs
cargo run --release --example bench -- <from-archive> <to-archive> [--registry NAME]
```

Turn the hook on once per clone with `git config core.hooksPath .githooks`. Do not skip it.

## Rules that are easy to break

- The crate has two public surfaces. Changing either one's signatures or output is a breaking change.
  - The `#[wasm_bindgen]` functions in `src/lib.rs` are the app's contract, tested by `tests/web.rs`. That includes the values they reject with: a failed download is an `Error` with `code`, `status` and `resetsAt`, as the README's "Errors" section lists.
  - The plain items `src/lib.rs` re-exports (`get_diff_content`, `build_patch`, `archive_source`, `unpack_archive`, …) are diffpack-server's contract, tested by `tests/public_api.rs`. The text `get_diff_content` and `build_patch` render is part of it, because the tree's line counts come from the same rendering.
- Host tests cannot call the `#[wasm_bindgen]` functions: creating a `JsValue` outside wasm32 aborts the process. Test them in `tests/web.rs`.
- A change here reaches no one until it is released and each consumer moves its pin (the npm version in diffpack, the `tag` and `engine::VERSION` in diffpack-server).
- The release profile (`opt-level = 's'` with `lto`) was picked by measuring with `examples/bench.rs`: about half the compute time of `'z'` for roughly 70 KB more. Measure again before changing it.
- `WASM_PACK_VERSION` is pinned in both `ci.yml` and `release.yml`; keep the two equal. The wasm-bindgen crates (`wasm-bindgen*`, `js-sys`, `web-sys`, `serde-wasm-bindgen`) move together.
- `deny.toml` allows crates.io dependencies with permissive licences only, no git dependencies.

## Releasing

1. Bump `version` in `Cargo.toml` (the only place it lives) and run `cargo check` so `Cargo.lock` follows. Commit as `chore(release): <version>`.
2. Merge into `development`, then merge `development` into `main`.
3. On `main`: `git tag v<version> && git push origin v<version>`. `release.yml` rejects a tag that differs from `Cargo.toml`, then publishes to npm.

## Git workflow

- Branch from `development` and open PRs against `development`. Only `main` releases.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `chore(deps):`, …).

## Code style

This is a firm rule, not a preference.

- Write self-explaining code in Clean Code style: intention-revealing names, small functions that do one thing, named constants and types instead of magic values, and well-named helper functions instead of blocks that need explaining.
- Do not add comments, doc comments included, to explain what code does or why it does it.
- If code seems to need a comment, rename or restructure it until it does not.
- The why behind a change belongs in the commit message or the PR description.
- There are no exceptions today: no lint in this crate requires doc comments or `// SAFETY:` notes.
