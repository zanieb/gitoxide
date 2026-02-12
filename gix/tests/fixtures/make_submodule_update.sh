#!/usr/bin/env bash
set -eu -o pipefail

# Creates a set of repos to test submodule init and update operations.
# Modelled after git's t7406-submodule-update.sh and libgit2's submodule/update.c tests.

git init -q upstream
(cd upstream
  echo "file" > file
  git add file
  git commit -q -m "upstream initial"
)

# Create a separate clone to advance independently.
git clone -q upstream submodule
(cd submodule
  echo "line2" > file
  git add file
  git commit -q -m "upstream commit 2"
)

# Create the superproject with one submodule.
# We add the submodule pointing to "submodule" (which has 2 commits).
git clone -q upstream super
(cd super
  git submodule add ../submodule submodule
  git commit -q -m "add submodule"
)

# --- Scenario: after-clone (not yet initialized) ---
# A fresh clone has submodule config in .gitmodules but nothing in .git/config
# and no submodule worktree.
git clone -q super after-clone

# --- Scenario: initialized-not-updated ---
# After `git submodule init`, the url is in .git/config but the worktree is empty.
git clone -q super initialized-not-updated
(cd initialized-not-updated
  git submodule init submodule
)

# --- Scenario: fully-updated ---
# After `git submodule update --init`, everything is checked out.
git clone -q super fully-updated
(cd fully-updated
  git submodule update --init
)

# --- Scenario: needs-update ---
# The superproject index points to a newer commit than the checked-out submodule.
# We advance the submodule in the superproject but leave the clone behind.
git clone -q super needs-update
(cd needs-update
  git submodule update --init
  # Reset the submodule to the older commit
  (cd submodule
    git checkout -q HEAD~1
  )
)

# --- Scenario: update-none ---
# Superproject that configures update=none via local config.
git clone -q super update-none
(cd update-none
  git submodule update --init
  git config submodule.submodule.update none
  # Reset to old so there's something to update
  (cd submodule
    git checkout -q HEAD~1
  )
)

# --- Scenario: dirty-submodule ---
# Submodule has local changes that should block checkout update.
git clone -q super dirty-submodule
(cd dirty-submodule
  git submodule update --init
  (cd submodule
    echo "local change" > file
  )
)

# --- Scenario: recursive ---
# A superproject with a submodule that itself has a submodule.
git init -q inner-module
(cd inner-module
  echo "inner" > inner-file
  git add inner-file
  git commit -q -m "inner initial"
)

git clone -q upstream mid-module
(cd mid-module
  git submodule add ../inner-module inner
  git commit -q -m "add inner submodule"
)

git init -q recursive-super
(cd recursive-super
  echo "top" > top-file
  git add top-file
  git commit -q -m "top initial"
  git submodule add ../mid-module mid
  git commit -q -m "add mid submodule"
)

# Fresh clone for --init --recursive testing
git clone -q recursive-super recursive-clone

# --- Scenario: command-in-gitmodules-rejected ---
# Superproject with update=!command in .gitmodules (must be rejected for security).
git clone -q super command-in-gitmodules
(cd command-in-gitmodules
  git config -f .gitmodules submodule.submodule.update '!false'
  git add .gitmodules
  git commit -q -m "add command update to .gitmodules"
)

# --- Scenario: init-no-overwrite ---
# Already-initialized submodule where init should not overwrite existing url.
git clone -q super init-no-overwrite
(cd init-no-overwrite
  git submodule init submodule
  # Override url to something custom
  git config submodule.submodule.url "https://custom.example.com/repo.git"
)
