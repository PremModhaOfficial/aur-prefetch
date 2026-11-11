#!/bin/sh
# Source this before any agent-driven command in this repo.
#
# Every hang here came from a program waiting for input that never arrives: a
# git commit editor, a heredoc with no terminator, a pager. These defaults make
# those fail fast instead of waiting forever.
#
#   . scripts/agent-env.sh
#   timeout 600 git rebase --continue </dev/null

export GIT_EDITOR=true
export GIT_SEQUENCE_EDITOR=true
export GIT_TERMINAL_PROMPT=0
export EDITOR=true
export VISUAL=true
export PAGER=cat
export GIT_PAGER=cat
export SYSTEMD_PAGER=cat
export DEBIAN_FRONTEND=noninteractive
