---
"pacquet": patch
---

pnpm can now read back a `node_modules/.modules.yaml` that holds a dependency path longer than 1024 characters. YAML caps a plain key at that length, and deep peer-dependency chains (Expo and React Native trees reach it) pass it, so pnpm rejected a file it had written itself and silently re-linked the whole tree on every install.
