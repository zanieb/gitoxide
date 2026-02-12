#!/usr/bin/env bash
set -eu -o pipefail

# This fixture creates a repository with various diff scenarios and
# captures C Git's diff output for each, so Rust tests can compare
# gix's patch formatter output against the reference.

git init -q -b main
git config user.name "Test"
git config user.email "test@example.com"

# ============================================================
# Scenario 1: Simple single-line modification
# ============================================================
printf "line 1\nline 2\nline 3\n" > simple.txt
git add simple.txt && git commit -q -m "add simple.txt"

printf "line 1\nline two\nline 3\n" > simple.txt
git add simple.txt && git commit -q -m "modify simple.txt"

git diff HEAD~1 HEAD -- simple.txt > expected_simple_modify.patch

# ============================================================
# Scenario 2: Add lines at end
# ============================================================
git checkout -q -b add-at-end HEAD~1
printf "line 1\nline 2\nline 3\n" > add_end.txt
git add add_end.txt && git commit -q -m "add add_end.txt"

printf "line 1\nline 2\nline 3\nline 4\nline 5\n" > add_end.txt
git add add_end.txt && git commit -q -m "add lines at end"

git diff HEAD~1 HEAD -- add_end.txt > expected_add_at_end.patch

# ============================================================
# Scenario 3: Remove lines from beginning
# ============================================================
git checkout -q -b remove-begin HEAD~1
printf "line 1\nline 2\nline 3\nline 4\n" > remove_begin.txt
git add remove_begin.txt && git commit -q -m "add remove_begin.txt"

printf "line 3\nline 4\n" > remove_begin.txt
git add remove_begin.txt && git commit -q -m "remove from beginning"

git diff HEAD~1 HEAD -- remove_begin.txt > expected_remove_begin.patch

# ============================================================
# Scenario 4: New file (added)
# ============================================================
git checkout -q -b new-file HEAD~1
# Create an empty commit first so HEAD~1 exists
git commit -q --allow-empty -m "empty base"

printf "brand new content\nline 2\n" > new_file.txt
git add new_file.txt && git commit -q -m "add new file"

git diff HEAD~1 HEAD -- new_file.txt > expected_new_file.patch

# ============================================================
# Scenario 5: Deleted file
# ============================================================
git checkout -q -b deleted-file HEAD~1
printf "to be deleted\nline 2\n" > deleted.txt
git add deleted.txt && git commit -q -m "add file to delete"

git rm -q deleted.txt && git commit -q -m "delete file"

git diff HEAD~1 HEAD -- deleted.txt > expected_deleted_file.patch

# ============================================================
# Scenario 6: No trailing newline
# ============================================================
git checkout -q -b no-newline HEAD~1
printf "has newline\n" > no_nl.txt
git add no_nl.txt && git commit -q -m "file with newline"

printf "no newline" > no_nl.txt
git add no_nl.txt && git commit -q -m "remove trailing newline"

git diff HEAD~1 HEAD -- no_nl.txt > expected_no_trailing_newline.patch

# ============================================================
# Scenario 7: Both files lack trailing newline
# ============================================================
git checkout -q -b both-no-newline HEAD~1
printf "old content" > both_no_nl.txt
git add both_no_nl.txt && git commit -q -m "file without newline"

printf "new content" > both_no_nl.txt
git add both_no_nl.txt && git commit -q -m "change content, still no newline"

git diff HEAD~1 HEAD -- both_no_nl.txt > expected_both_no_newline.patch

# ============================================================
# Scenario 8: Multiple hunks
# ============================================================
git checkout -q -b multi-hunk HEAD~1
# Create a file with enough lines so changes are in separate hunks
for i in $(seq 1 30); do
    echo "line $i" >> multi.txt
done
git add multi.txt && git commit -q -m "add multi.txt"

# Modify lines near beginning and end (far enough apart for separate hunks)
sed -i 's/^line 3$/changed line 3/' multi.txt
sed -i 's/^line 28$/changed line 28/' multi.txt
git add multi.txt && git commit -q -m "two separate changes"

git diff HEAD~1 HEAD -- multi.txt > expected_multi_hunk.patch

# ============================================================
# Scenario 9: Binary file
# ============================================================
git checkout -q -b binary-file HEAD~1
git commit -q --allow-empty -m "empty base for binary"

printf '\x00\x01\x02binary' > binary.bin
git add binary.bin && git commit -q -m "add binary file"

git diff HEAD~1 HEAD -- binary.bin > expected_binary_file.patch

# ============================================================
# Scenario 10: Mode change (644 -> 755) with content change
# ============================================================
git checkout -q -b mode-change HEAD~1
printf "#!/bin/sh\necho hello\n" > script.sh
git add script.sh && git commit -q -m "add script"

chmod 755 script.sh
printf "#!/bin/sh\necho world\n" > script.sh
git add script.sh && git commit -q -m "make executable and change content"

git diff HEAD~1 HEAD -- script.sh > expected_mode_change.patch

# ============================================================
# Scenario 11: Function name in hunk header (C-like)
# ============================================================
git checkout -q -b func-name HEAD~1
cat > func.c <<'CEOF'
#include <stdio.h>

int helper(int x) {
    return x + 1;
}

int main(void) {
    int a = 0;
    int b = 0;
    int c = 0;
    int d = 0;
    printf("hello");
    return 0;
}
CEOF
git add func.c && git commit -q -m "add func.c"

cat > func.c <<'CEOF'
#include <stdio.h>

int helper(int x) {
    return x + 1;
}

int main(void) {
    int a = 0;
    int b = 0;
    int c = 0;
    int d = 0;
    printf("world");
    return 0;
}
CEOF
git add func.c && git commit -q -m "change in main function"

git diff HEAD~1 HEAD -- func.c > expected_func_name.patch

# ============================================================
# Scenario 12: Entire file replaced (all old lines removed, all new lines added)
# ============================================================
git checkout -q -b full-replace HEAD~1
printf "old line 1\nold line 2\nold line 3\n" > replaced.txt
git add replaced.txt && git commit -q -m "add replaced.txt"

printf "new line A\nnew line B\nnew line C\nnew line D\n" > replaced.txt
git add replaced.txt && git commit -q -m "replace entire content"

git diff HEAD~1 HEAD -- replaced.txt > expected_full_replace.patch

# ============================================================
# Scenario 13: Add trailing newline
# ============================================================
git checkout -q -b add-newline HEAD~1
printf "no newline" > add_nl.txt
git add add_nl.txt && git commit -q -m "file without newline"

printf "no newline\n" > add_nl.txt
git add add_nl.txt && git commit -q -m "add trailing newline"

git diff HEAD~1 HEAD -- add_nl.txt > expected_add_newline.patch

# ============================================================
# Scenario 14: Empty file to content
# ============================================================
git checkout -q -b empty-to-content HEAD~1
touch empty_to_content.txt
git add empty_to_content.txt && git commit -q -m "add empty file"

printf "now has content\n" > empty_to_content.txt
git add empty_to_content.txt && git commit -q -m "add content to empty file"

git diff HEAD~1 HEAD -- empty_to_content.txt > expected_empty_to_content.patch

# ============================================================
# Scenario 15: Rename detection
# ============================================================
git checkout -q -b rename-detect HEAD~1
printf "original content\nline 2\nline 3\n" > before_rename.txt
git add before_rename.txt && git commit -q -m "add before_rename.txt"

git mv before_rename.txt after_rename.txt
git commit -q -m "rename file"

git diff -M HEAD~1 HEAD > expected_rename.patch

# Go back to main branch
git checkout -q main
