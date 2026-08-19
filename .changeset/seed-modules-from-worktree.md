---
"pacquet": minor
---

New setting `seedModulesFromWorktree`. When it is on and `node_modules` is missing, pnpm clones one from another git worktree of the same repository and installs only what differs, instead of writing the whole tree from the store. On a 3,582-package React Native monorepo a fresh worktree installs in 13 s instead of 44. It only clones from a worktree whose lockfile, workspace manifest and project manifests match, and whose own tree is up to date, so the install that follows has nothing to redo. Copy-on-write keeps the two trees independent. Needs a filesystem that clones a directory in one operation, which today means APFS; everywhere else the setting does nothing. Off by default.
