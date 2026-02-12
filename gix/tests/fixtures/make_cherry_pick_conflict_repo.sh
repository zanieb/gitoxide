#!/usr/bin/env bash
set -eu -o pipefail

git init -q -b main

# Initial commit on main: file.txt = "initial"
echo "initial" > file.txt
git add file.txt
git commit -m "initial commit"

# Create a feature branch that modifies file.txt in a conflicting way
git checkout -b conflict-feature
echo "conflict feature change" > file.txt
git add file.txt
git commit -m "conflict-feature: modify file.txt"

# Go back to main and make a conflicting change to the same file
git checkout main
echo "main conflicting change" > file.txt
git add file.txt
git commit -m "main: conflicting change to file.txt"
