---
"pacquet": patch
---

Fixed an install failure under `nodeLinker: hoisted` when a dependency publishes a `node_modules` directory inside its own tarball, such as `@parcel/watcher-wasm`. Re-importing the package failed the whole install; its shipped directory is now merged with the one the linker installed alongside it.
