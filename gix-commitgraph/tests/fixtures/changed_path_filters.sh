#!/usr/bin/env bash
set -eu -o pipefail

git init -q

echo base >base.txt
git add base.txt
git commit -q -m base
git branch base

mkdir dir
echo child >dir/child.txt
git add dir/child.txt
git commit -q -m child
git branch child

git commit-graph write --no-progress --reachable --changed-paths
