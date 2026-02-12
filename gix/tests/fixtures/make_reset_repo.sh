#!/usr/bin/env bash
set -eu -o pipefail

git init -q

git checkout -b main

# Create initial commit (c1)
echo "first file" > first
git add first
git commit -q -m "create first file"

# Create second commit (c2)
echo "second file" > second
git add second
git commit -q -m "create second file"

# Create third commit (c3) - modify first
echo "modified first" > first
git commit -q -am "modify first file"
