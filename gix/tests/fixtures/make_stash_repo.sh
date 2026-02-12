#!/usr/bin/env bash
set -eu -o pipefail

git init -q

git checkout -b main

# Create initial commit
echo "hello" > file.txt
git add file.txt
git commit -q -m "initial commit"

# Stage a modification (so the index differs from HEAD)
echo "modified" > file.txt
git add file.txt
