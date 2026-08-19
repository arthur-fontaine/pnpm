---
"pacquet": patch
---

A package's `files` entries now name paths from the package root, matching npm. A bare `src` published the root `src` directory and anything named `src` deeper in the tree, so a git dependency shipped the repository's own example app: one React Native monorepo installed 13,324 files where 45 were published. Exclusions such as `!**/__tests__` and `!*.map` still match at any depth.
