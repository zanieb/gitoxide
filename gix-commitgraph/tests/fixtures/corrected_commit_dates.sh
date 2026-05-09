#!/usr/bin/env bash
set -eu -o pipefail

git init -q

export GIT_AUTHOR_DATE="2000-01-01T00:00:00 +0000"
export GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"

git checkout -q -b parent
git commit -q --allow-empty -m parent

git checkout -q -b child parent
git commit -q --allow-empty -m child

git -c commitGraph.generationVersion=2 commit-graph write --no-progress --reachable
