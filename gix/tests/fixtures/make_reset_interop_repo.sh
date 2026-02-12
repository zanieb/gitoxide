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

# Create a branch for merge tests
git branch side HEAD~1

# Create fourth commit (c4) on main - add third file
echo "third file" > third
git add third
git commit -q -m "add third file"

# Create a commit on side branch for potential merge conflict
git checkout -q side
echo "side change to first" > first
git commit -q -am "side: modify first"
git checkout -q main
