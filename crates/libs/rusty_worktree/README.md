# rusty_worktree

Git worktrees that share one repository's object store. Each task or card
gets its own working tree under a directory the caller chooses, branched or
detached from a base, while objects are stored once in the repository.

Every command runs the `git` CLI with `GIT_*` variables removed and hooks
off, and every command that acts on a worktree names its git directory
explicitly, so a rewritten `.git` file inside a worktree cannot point the
caller somewhere else. That matters when untrusted code edits the worktree:
a sandboxed process may write the worktree, read `Repo::git_dir`, and leave
staging and commits to the caller.

Used by mill-runner (a worktree per card) and by BBP's confined harnesses
(a worktree per turn).
