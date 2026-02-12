#!/usr/bin/env bash
set -eu -o pipefail

# Creates a repo with git notes for integration testing.
#
# Layout:
#   - 3 commits on main (A, B, C)
#   - Notes on commits A and B (refs/notes/commits)
#   - A custom note on commit C (refs/notes/custom)
#   - No note on commit C in default ref

git init -q -b main
git config user.name "Test"
git config user.email "test@example.com"

# Commit A
echo "first" > file.txt
git add file.txt
git commit -q -m "commit A"

# Commit B
echo "second" > file.txt
git add file.txt
git commit -q -m "commit B"

# Commit C
echo "third" > file.txt
git add file.txt
git commit -q -m "commit C"

# Add notes to commits A and B in default ref
COMMIT_A=$(git rev-parse HEAD~2)
COMMIT_B=$(git rev-parse HEAD~1)
COMMIT_C=$(git rev-parse HEAD)

git notes add -m "Note for commit A" "$COMMIT_A"
git notes add -m "Note for commit B" "$COMMIT_B"

# Add a custom note on commit C in a different ref
git notes --ref=refs/notes/custom add -m "Custom note for C" "$COMMIT_C"
