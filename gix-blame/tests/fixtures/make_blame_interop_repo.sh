#!/usr/bin/env bash
set -eu -o pipefail

# Interop test repo: exercises scenarios from C Git's blame tests
# (t8003-blame-corner-cases.sh, t8013-blame-ignore-revs.sh, t8009-blame-vs-topicbranches.sh)
# and provides baselines captured with `git blame --porcelain`.

git init -q
git config --local diff.algorithm histogram
git config merge.ff false
git checkout -q -b main

# ============================================================
# Scenario 1 — ignore-revs: A adds line1, B adds line2, X modifies both
# ============================================================
echo "line1" > ignore-revs-file.txt
git add ignore-revs-file.txt
GIT_AUTHOR_NAME="Author-A" git commit -q -m A
git tag A

echo "line2" >> ignore-revs-file.txt
git add ignore-revs-file.txt
GIT_AUTHOR_NAME="Author-B" git commit -q -m B
git tag B

printf "line-one\nline-two\n" > ignore-revs-file.txt
git add ignore-revs-file.txt
GIT_AUTHOR_NAME="Author-X" git commit -q -m X
git tag X

# Baselines for ignore-revs scenario
git blame --porcelain ignore-revs-file.txt > .git/ignore-revs-file.baseline
git blame --porcelain --ignore-rev "$(git rev-parse X)" ignore-revs-file.txt > .git/ignore-revs-file-ignore-X.baseline

# ============================================================
# Scenario 2 — ignore-revs with added "unblamable" lines
# Y modifies lines 1-2 and adds lines 3-4
# ============================================================
printf "line-one-change\nline-two-changed\ny3\ny4\n" > ignore-revs-file.txt
git add ignore-revs-file.txt
GIT_AUTHOR_NAME="Author-Y" git commit -q -m Y
git tag Y

git blame --porcelain ignore-revs-file.txt > .git/ignore-revs-file-with-Y.baseline
git blame --porcelain --ignore-rev "$(git rev-parse Y)" ignore-revs-file.txt > .git/ignore-revs-file-ignore-Y.baseline

# ============================================================
# Scenario 3 — multiple authors, each line from a different commit
# ============================================================
echo "alpha" > multi-author.txt
git add multi-author.txt
GIT_AUTHOR_NAME="Alice" git commit -q -m "multi-author-1"

echo "beta" >> multi-author.txt
git add multi-author.txt
GIT_AUTHOR_NAME="Bob" git commit -q -m "multi-author-2"

echo "gamma" >> multi-author.txt
git add multi-author.txt
GIT_AUTHOR_NAME="Charlie" git commit -q -m "multi-author-3"

echo "delta" >> multi-author.txt
git add multi-author.txt
GIT_AUTHOR_NAME="Diana" git commit -q -m "multi-author-4"

echo "epsilon" >> multi-author.txt
git add multi-author.txt
GIT_AUTHOR_NAME="Eve" git commit -q -m "multi-author-5"

git blame --porcelain multi-author.txt > .git/multi-author.baseline

# ============================================================
# Scenario 4 — merge from topic branch (ported from t8009)
# ============================================================
printf "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n" > topicbranch.txt
git add topicbranch.txt
git commit -q -m "base for topic branch"
git tag topic-base

git checkout -b topic
printf "one\ntwo\nthree\nfour modified on topic\nfive\nsix\nseven\neight\n" > topicbranch.txt
git add topicbranch.txt
git commit -q -m "modify line 4 on topic"
git tag topic-change

git checkout main
printf "one\ntwo modified on main\nthree\nfour\nfive\nsix\nseven\neight\n" > topicbranch.txt
git add topicbranch.txt
git commit -q -m "modify line 2 on main"
git tag main-change

git merge topic -m "merge topic into main" || true

git blame --porcelain topicbranch.txt > .git/topicbranch.baseline

# ============================================================
# Scenario 5 — empty file
# ============================================================
touch empty-file.txt
git add empty-file.txt
git commit -q -m "add empty file"

git blame --porcelain empty-file.txt > .git/empty-file.baseline || true

# ============================================================
# Scenario 6 — blame at a specific (non-HEAD) revision
# ============================================================
# We use the tag "B" to blame ignore-revs-file.txt at that point
git blame --porcelain B -- ignore-revs-file.txt > .git/ignore-revs-file-at-B.baseline

# ============================================================
# Scenario 7 — coalesce test from t8003: add a SPLIT line then remove it
# ============================================================
printf "ABC\nDEF\n" > coalesce-interop.txt
git add coalesce-interop.txt
git commit -q -m "coalesce-original"
git tag coalesce-orig

printf "ABC\nSPLIT\nDEF\n" > coalesce-interop.txt
git add coalesce-interop.txt
git commit -q -m "coalesce-split"
git tag coalesce-split

printf "ABC\nDEF\n" > coalesce-interop.txt
git add coalesce-interop.txt
git commit -q -m "coalesce-final"
git tag coalesce-final

git blame --porcelain coalesce-interop.txt > .git/coalesce-interop.baseline

# ============================================================
# Scenario 8 — file that was a directory then became a file (t8003)
# ============================================================
mkdir path-was-dir
echo "content A" > path-was-dir/file
echo "content B" > path-was-dir/elif
git add path-was-dir
git commit -q -m "path was a directory"
git tag dir-commit

rm -rf path-was-dir
echo "content A" > path-was-dir
git add path-was-dir
git commit -q -m "path is now a regular file"

git blame --porcelain path-was-dir > .git/path-was-dir.baseline

# ============================================================
# Scenario 9 — ignore-revs: boundary checks with negative parent size
# From t8013: A--B--C, ignore B to test boundary checks
# ============================================================
printf "L1\nL2\nL7\nL8\nL9\n" > boundary-check.txt
git add boundary-check.txt
git commit -q -m "boundary-A"
git tag boundary-A

printf "L1\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nL9\n" > boundary-check.txt
git add boundary-check.txt
git commit -q -m "boundary-B"
git tag boundary-B

printf "L1\nL2\nL3\nL4\nxxx\nL6\nL7\nL8\nL9\n" > boundary-check.txt
git add boundary-check.txt
git commit -q -m "boundary-C"
git tag boundary-C

git blame --porcelain boundary-check.txt > .git/boundary-check.baseline
git blame --porcelain --ignore-rev "$(git rev-parse boundary-B)" boundary-check.txt > .git/boundary-check-ignore-B.baseline

# ============================================================
# Scenario 10 — ignore merge: A--B--M and A--C--M
# ============================================================
printf "L1\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nL9\n" > ignore-merge.txt
git add ignore-merge.txt
git commit -q -m "ignore-merge-A"
git tag merge-A

printf "BB\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nL9\n" > ignore-merge.txt
git add ignore-merge.txt
git commit -q -m "ignore-merge-B"
git tag merge-B

git checkout -b merge-branch-c
git reset --hard merge-A
printf "L1\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nCC\n" > ignore-merge.txt
git add ignore-merge.txt
git commit -q -m "ignore-merge-C"
git tag merge-C

git checkout main
git merge merge-branch-c -m "merge M" || true
git tag merge-M

git blame --porcelain ignore-merge.txt > .git/ignore-merge.baseline
git blame --porcelain --ignore-rev "$(git rev-parse merge-M)" ignore-merge.txt > .git/ignore-merge-ignore-M.baseline

# ============================================================
# Scenario 11 — blame with line range (-L)
# ============================================================
git blame --porcelain -L 3,5 multi-author.txt > .git/multi-author-L3-5.baseline
