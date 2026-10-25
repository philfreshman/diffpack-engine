#!/bin/sh

set -eu

release_version=$1
crate_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)

if [ "$crate_version" != "$release_version" ]; then
    echo "::error::The commits since the last release call for version $release_version, but Cargo.toml says $crate_version. Nothing was tagged or published. Set version = \"$release_version\" in Cargo.toml in a chore(release): $release_version pull request to development, then merge development into main again."
    exit 1
fi

echo "Cargo.toml says $crate_version, as the commits call for."
