# CLAUDE.md

diffpack-engine fetches, extracts and diffs package archives (npm, crates.io, PyPI, Packagist/Composer, Go). It ships twice: as a wasm module on npm (`@philfreshman/diffpack-engine`) for the diffpack web app, and as a Cargo git dependency, pinned by tag, for diffpack-server. Cross-repo context is in `../CLAUDE.md`.

## Commands

```bash
cargo test                                   # host-side suite, fast and offline
cargo test <name>                            # tests whose name contains <name>
cargo test --test public_api                 # the Rust surface as diffpack-server sees it
wasm-pack test --headless --chrome           # the wasm bindings in a real browser, against crates.io
DIFFPACK_PROXY_ORIGIN=https://diffpack-proxy.philipswiezak.workers.dev wasm-pack test --headless --chrome --test web_live_proxy -- --include-ignored   # one real Composer comparison through the proxy; --ignored does not run it in the browser
wasm-pack build --release --target web --scope philfreshman   # writes pkg/ (gitignored, never commit it)
.githooks/pre-commit                         # fmt, clippy -D warnings, cargo deny, cargo audit: what CI runs
cargo run --release --example bench -- <from-archive> <to-archive> [--registry NAME]
```

Turn the hook on once per clone with `git config core.hooksPath .githooks`. Do not skip it.

## Rules that are easy to break

- The crate has two public surfaces. Changing either one's signatures or output is a breaking change.
  - The `#[wasm_bindgen]` functions in `src/lib.rs` are the app's contract, tested by `tests/web*.rs`. `tests/web_without_proxy.rs` is its own page so no proxy origin is set there. That includes the values they reject with: a failed download is an `Error` with `code`, `status` and `resetsAt`, as the README's "Errors" section lists.
  - The plain items `src/lib.rs` re-exports (`get_diff_content`, `build_patch`, `archive_source`, `unpack_archive`, …) are diffpack-server's contract, tested by `tests/public_api.rs`. The text `get_diff_content` and `build_patch` render is part of it, because the tree's line counts come from the same rendering. `choose_archive(registry, version, listing)` takes the version since 0.5.0, so Composer can pick one version out of Packagist's listing of all of them.
- Host tests cannot call the `#[wasm_bindgen]` functions: creating a `JsValue` outside wasm32 aborts the process. Test them in `tests/web.rs`.
- In wasm, Composer goes through diffpack-proxy (`set_proxy_origin`, `src/proxy.rs`); native code uses the direct URLs. The proxy's HTTP contract is `../diffpack-proxy/docs/contract.md`.
- A change here reaches no one until it is released and each consumer moves its pin (the npm version in diffpack, the `tag` and `engine::VERSION` in diffpack-server).
- The release profile (`opt-level = 's'` with `lto`) was picked by measuring with `examples/bench.rs`: about half the compute time of `'z'` for roughly 70 KB more. Measure again before changing it.
- `WASM_PACK_VERSION` is pinned in both `ci.yml` and `release.yml`; keep the two equal. The wasm-bindgen crates (`wasm-bindgen*`, `js-sys`, `web-sys`, `serde-wasm-bindgen`) move together.
- `deny.toml` allows crates.io dependencies with permissive licences only, no git dependencies.

## Releasing

Every merge into `main` runs `release.yml`. It reads the Conventional Commits since the last `v*` tag and works out the next version from them. If there is one, it publishes the package to npm, pushes the tag `v<version>` and writes the GitHub release with notes and `diffpack_engine_bg.wasm` attached. Nothing is committed back to `main`, so `Cargo.toml` stays the only place the version lives and must already hold the new version when the merge lands.

The crate stays on 0.x, so a breaking change moves the minor number, the way Cargo reads 0.x versions:

| Commits since the last tag | Next version, from 0.5.0 |
| :- | :- |
| a breaking change: `!` after the type (`feat!:`), or a `BREAKING CHANGE:` footer | 0.6.0 |
| `feat`, `fix`, `perf`, or a revert | 0.5.1 |
| only `bench`, `build`, `chore`, `ci`, `docs`, `refactor`, `style` or `test` | no release |

The rules live in `.releaserc.json`. To release:

1. Find the version the commits call for. This simulates the merge into `main` in a throwaway clone and asks semantic-release, without publishing anything:

   ```bash
   cd "$(mktemp -d)" && git clone -q -b main https://github.com/philfreshman/diffpack-engine.git .
   git merge -q --no-ff -m check origin/development
   GITHUB_TOKEN=$(gh auth token) npx -y -p semantic-release@25 -p @semantic-release/exec@7 -p conventional-changelog-conventionalcommits@9 semantic-release --dry-run --no-ci
   ```

   Before the bump it fails with the version the commits call for. After the bump it prints that version and the release notes. With no releasable commits it says no new version is released, and there is nothing to bump.
2. Set `version` in `Cargo.toml` to it and run `cargo check` so `Cargo.lock` follows. Commit as `chore(release): <version>`, open the pull request against `development` and merge it.
3. Open a pull request from `development` to `main` and merge it with a merge commit. `release.yml` does the rest.
4. diffpack-server then moves its `tag = "v<version>"` in `Cargo.toml` and `engine::VERSION` together. diffpack gets the new npm version from Renovate.

`release.yml` builds `pkg/` with wasm-pack before anything else, so a build that fails stops the release before a tag exists. semantic-release then goes through these steps in order, and the first one that fails stops the rest:

1. `.github/scripts/check-release-version.sh` compares the computed version with `Cargo.toml`. If they differ, the job fails and says which version the commits call for and what `Cargo.toml` says.
2. `.github/scripts/publish-npm.sh` runs `npm publish --access public --provenance` from `pkg/`, unless that version is already on npm.
3. semantic-release pushes the tag.
4. semantic-release writes the GitHub release and uploads the `.wasm`.

A failed release opens an issue in this repository. To recover:

- Step 1 failed: nothing was published or tagged. Bump `Cargo.toml` to the version the error names in a `chore(release)` pull request to `development`, then merge `development` into `main` again.
- The build or step 2 failed (for example an expired `NPM_TOKEN` secret): nothing was published or tagged. Fix the cause and re-run the job.
- Step 3 failed: the version is on npm but has no tag. Re-run the job. Step 2 sees the version on npm and skips it, then the tag and the release follow.
- Step 4 failed: the version is on npm and tagged, and a re-run releases nothing because semantic-release sees the tag. Create the release by hand with the `.wasm` from the npm tarball: `npm pack @philfreshman/diffpack-engine@<version> && tar -xzf philfreshman-diffpack-engine-<version>.tgz && gh release create v<version> --verify-tag --title v<version> --generate-notes package/diffpack_engine_bg.wasm`.

npm never lets a version be published twice, so a broken release is fixed with a new version, not by publishing again.

## Git workflow

- Branch from `development` and open PRs against `development`. `main` only takes merges from `development` (the `main source is development` check), and every merge into `main` may release.
- Use Conventional Commits: `type(scope): description` with the type one of `bench`, `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `style` or `test`, the scope optional, and `!` before the colon for a breaking change. The `conventional commits` check fails a pull request with any other commit subject. The type decides the next version, as "Releasing" describes.

## Code style

This is a firm rule, not a preference.

- Write self-explaining code in Clean Code style: intention-revealing names, small functions that do one thing, named constants and types instead of magic values, and well-named helper functions instead of blocks that need explaining.
- Do not add comments, doc comments included, to explain what code does or why it does it.
- If code seems to need a comment, rename or restructure it until it does not.
- The why behind a change belongs in the commit message or the PR description.
- There are no exceptions today: no lint in this crate requires doc comments or `// SAFETY:` notes.
