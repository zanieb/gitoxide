#!/usr/bin/env bash
set -eu -o pipefail

git init -q
git config commitGraph.generationVersion 2

git checkout -q -b parent
export GIT_AUTHOR_DATE="@4102444800 +0000"
export GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"
git commit -q --allow-empty -m parent

git checkout -q -b child parent
export GIT_AUTHOR_DATE="@946684800 +0000"
export GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"
git commit -q --allow-empty -m child

git commit-graph write --no-progress --reachable
