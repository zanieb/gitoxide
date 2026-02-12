#!/usr/bin/env bash
set -eu -o pipefail

git init -q -b main

# Initial commit on main: file.txt = "initial"
echo "initial" > file.txt
git add file.txt
git commit -m "initial commit"

# Create a feature branch with a commit to cherry-pick
git checkout -b feature
echo "feature change" > file.txt
git add file.txt
git commit -m "feature: modify file.txt"

# Add another file on feature branch for a second commit
echo "new file content" > new_file.txt
git add new_file.txt
git commit -m "feature: add new_file.txt"

# Go back to main and make a non-conflicting change
git checkout main
echo "other content" > other.txt
git add other.txt
git commit -m "main: add other.txt"

# === Create a merge commit on a separate branch ===
# We'll create two branches from initial commit, then merge them.
git checkout -b merge-base HEAD~1   # go back to "initial commit"
echo "merge-base content" > merge-base.txt
git add merge-base.txt
git commit -m "merge-base: add merge-base.txt"

git checkout -b merge-side HEAD~1   # go back to "initial commit"
echo "merge-side content" > merge-side.txt
git add merge-side.txt
git commit -m "merge-side: add merge-side.txt"

# Merge merge-base into merge-side to create a merge commit
git merge merge-base -m "merge: combine base and side"

# Tag the merge commit for easy reference
git tag merge-commit HEAD

# === Create an orphan branch with a root commit for root-commit cherry-pick test ===
git checkout --orphan orphan-root
git rm -rf .
echo "orphan content" > orphan.txt
git add orphan.txt
git commit -m "orphan: root commit with orphan.txt"
git tag root-commit HEAD

# Go back to main for the test starting point
git checkout main
