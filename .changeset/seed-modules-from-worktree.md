---
"pacquet": minor
---

A fresh git worktree no longer writes its whole `node_modules` from the store. When `node_modules` is missing, pnpm clones one from another worktree of the same repository and installs only what differs. On a 3,582-package React Native monorepo a fresh worktree installs in 13 s instead of 44. It only clones from a worktree whose lockfile, workspace manifest and project manifests match, and whose own tree is up to date, so the install that follows has nothing to redo. Copy-on-write keeps the two trees independent. Needs a filesystem that clones a directory in one operation, which today means APFS; everywhere else installs are unchanged.
