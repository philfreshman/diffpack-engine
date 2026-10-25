#!/bin/sh

set -eu

release_version=$1
package=$(node -p 'require("./pkg/package.json").name')

if [ "$(npm view "$package@$release_version" version 2>/dev/null)" = "$release_version" ]; then
    echo "$package@$release_version is already on npm, so it is not published again."
    exit 0
fi

cd pkg
npm publish --access public --provenance
