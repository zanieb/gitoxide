#!/usr/bin/env bash
set -eu

# Create a bare "remote" repository
git init --bare remote.git

# Create a working repository with commits
git init working
(
  cd working
  git config user.name "Test User"
  git config user.email "test@example.com"

  # First commit
  echo "initial" > file1.txt
  git add file1.txt
  git commit -m "first commit"

  # Second commit
  echo "second" > file2.txt
  git add file2.txt
  git commit -m "second commit"

  # Create a feature branch
  git checkout -b feature
  echo "feature work" > feature.txt
  git add feature.txt
  git commit -m "feature commit"

  # Create a tag
  git checkout main
  git tag v1.0

  # Third commit (only on main, after tag)
  echo "third" > file3.txt
  git add file3.txt
  git commit -m "third commit"

  # Create a second tag on the latest commit
  git tag v2.0
)
