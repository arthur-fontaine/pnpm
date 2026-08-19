---
"pacquet": patch
---

Under `nodeLinker: hoisted`, two peer variants of the same package version now share one directory instead of getting a nested copy each. A workspace where two projects depend on the same package but resolve one of its peers differently installed that package twice; a React Native monorepo carried five identical copies of `react-native`, 4,523 files each.
