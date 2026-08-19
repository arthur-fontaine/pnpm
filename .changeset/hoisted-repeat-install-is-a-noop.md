---
"pacquet": patch
---

Under `nodeLinker: hoisted`, a repeat `pnpm install` is a no-op again when a workspace package declares the dependencies. The hoisted linker installs every workspace package's dependencies into the root `node_modules`, but the up-to-date check looked for them under each package's own `node_modules`, found nothing there, and re-imported the whole tree on every install ([#14001](https://github.com/pnpm/pnpm/issues/14001)). A hoisted install also no longer logs `pnpm:_broken_node_modules` for the virtual-store paths it never writes.
