# Native migration fixture

`v1-square.cdraft` is a saved ZIP/RON file using the document-only version1 wrapper and core document schema1. It was created for the M6 migration contract; M5 had schema DTOs but no released native encoder. The test loads this on-disk file directly, checks the source version and geometry, then saves/reloads version2 with identical document values.

Reproduce from the workspace root:

```sh
cargo run -p cindral-io --example make_v1_fixture
```

Normal save/autosave APIs always emit version2. The fixture generator is separate so changing current serialization does not silently regenerate a migration test's input.
