#!/bin/bash
# Runs one long work package as a separate headless Claude Code session (its own prompt cache,
# unlike a subagent) in its own git worktree and branch.
#   tools/claude_session.sh <name> <model> <effort> <prompt-file>
# e.g. tools/claude_session.sh wp52-foo claude-sonnet-5-5 high prompt.md
# The transcript goes to .claude/sessions/<name>.jsonl; the last line with "type":"result" is the
# final report. Resume a stopped session with: claude -p --resume <session_id> "continue".
set -u
name="$1"; model="$2"; effort="$3"; prompt="$4"
repo="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
wt="$repo/.claude/worktrees/$name"
mkdir -p "$repo/.claude/sessions"
if [ ! -d "$wt" ]; then
  git -C "$repo" worktree add -q -b "$name" "$wt" main 2>/dev/null || git -C "$repo" worktree add -q "$wt" "$name"
fi
cd "$wt"
claude -p --model "$model" --effort "$effort" --permission-mode bypassPermissions \
  --output-format stream-json --verbose < "$prompt" > "$repo/.claude/sessions/$name.jsonl" 2>&1
echo "session $name exited with $?"
