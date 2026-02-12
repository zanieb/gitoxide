#!/usr/bin/env bash
set -eu -o pipefail

git init -q -b main

# Configure author/committer for reproducible commits.
git config user.name "Test Author"
git config user.email "test@example.com"

# === Initial commit on main ===
cat >file.txt <<'EOF'
line 1
line 2
line 3
line 4
line 5
EOF
echo "base content" > base.txt
git add file.txt base.txt
git commit -m "initial commit"

# === Feature branch: a clean cherry-pickable change ===
git checkout -b feature
echo "new feature file" > feature.txt
git add feature.txt
git commit -m "feature: add feature.txt"

# Second feature commit: modify file.txt
cat >file.txt <<'EOF'
line 1
line 2 modified by feature
line 3
line 4
line 5
EOF
git add file.txt
git commit -m "feature: modify file.txt line 2"

# === Go back to main, add non-conflicting change ===
git checkout main
echo "main extra" > main_extra.txt
git add main_extra.txt
git commit -m "main: add main_extra.txt"

# === Conflict branch: touches the same lines as main ===
git checkout -b conflict-branch main
cat >file.txt <<'EOF'
line 1
line 2 conflict version
line 3
line 4
line 5
EOF
git add file.txt
git commit -m "conflict-branch: modify file.txt line 2"

# Go back to main and make a conflicting change
git checkout main
cat >file.txt <<'EOF'
line 1
line 2 main version
line 3
line 4
line 5
EOF
git add file.txt
git commit -m "main: modify file.txt line 2"

# === Rename branch: tests rename tracking ===
git checkout -b rename-branch main~1
git mv file.txt renamed.txt
git commit -m "rename-branch: rename file.txt to renamed.txt"

# === Multi-commit branch for sequential cherry-pick ===
git checkout -b multi main~2
echo "multi-1" > multi1.txt
git add multi1.txt
git commit -m "multi: add multi1.txt"

echo "multi-2" > multi2.txt
git add multi2.txt
git commit -m "multi: add multi2.txt"

echo "multi-3" > multi3.txt
git add multi3.txt
git commit -m "multi: add multi3.txt"

# Return to main
git checkout main

# Save branch tips as tags for easy reference in tests.
git tag feature-tip feature
git tag feature-parent feature~1
git tag conflict-tip conflict-branch
git tag rename-tip rename-branch
git tag multi-1 multi~2
git tag multi-2 multi~1
git tag multi-3 multi
