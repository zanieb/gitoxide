#!/usr/bin/env bash
set -eu -o pipefail

git init -q

git checkout -b main
touch file
git add file
git commit -q -m "initial"

# Create a pre-commit hook that succeeds
cat > .git/hooks/pre-commit << 'HOOK'
#!/bin/sh
echo "pre-commit ran"
exit 0
HOOK
chmod +x .git/hooks/pre-commit

# Create a commit-msg hook that succeeds
cat > .git/hooks/commit-msg << 'HOOK'
#!/bin/sh
echo "commit-msg ran with: $1"
exit 0
HOOK
chmod +x .git/hooks/commit-msg

# Create a post-commit hook
cat > .git/hooks/post-commit << 'HOOK'
#!/bin/sh
echo "post-commit ran"
exit 0
HOOK
chmod +x .git/hooks/post-commit

# Create a hook that fails
cat > .git/hooks/pre-push << 'HOOK'
#!/bin/sh
echo "pre-push: rejecting push"
exit 1
HOOK
chmod +x .git/hooks/pre-push

# Create a hook that reads stdin
cat > .git/hooks/pre-receive << 'HOOK'
#!/bin/sh
while read line; do
    echo "received: $line"
done
exit 0
HOOK
chmod +x .git/hooks/pre-receive

# Create a non-executable hook (should be ignored)
cat > .git/hooks/post-update << 'HOOK'
#!/bin/sh
echo "should not run"
HOOK
# Intentionally NOT chmod +x

# Create a repo with custom hooks path
git init -q custom-hooks-path
(cd custom-hooks-path
  git checkout -b main
  touch file
  git add file
  git commit -q -m "initial"

  mkdir -p ../custom-hooks
  cat > ../custom-hooks/pre-commit << 'HOOK'
#!/bin/sh
echo "custom hooks path pre-commit"
exit 0
HOOK
  chmod +x ../custom-hooks/pre-commit

  git config core.hooksPath ../custom-hooks
)
