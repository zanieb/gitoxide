#!/usr/bin/env bash
set -eu -o pipefail

# Create a repository suitable for testing worktree add/remove operations

mkdir repo
(
  cd repo
  git init -q

  git checkout -b main
  mkdir dir
  echo "content a" > a
  echo "content b" > b
  echo "content c" > dir/c
  git add .
  git commit -q -m "initial commit"

  echo "updated a" >> a
  git commit -q -am "second commit"

  # Create some branches for testing
  git branch feature-1
  git branch feature-2
  git branch locked-branch
)

# Also create a bare repository variant
git clone --bare --shared repo repo.git
