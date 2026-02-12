#!/usr/bin/env bash
set -eu

git init repo
(
  cd repo
  git config user.name "Test User"
  git config user.email "test@example.com"
  git config core.abbrev 12

  echo "hello" > file.txt
  git add file.txt
  git commit -m "initial commit"
)
