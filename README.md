# json-toml-worm

A larvae worm that makes `require("./thing.json")` and
`require("./config.toml")` work, in the build and in the editor, without
larvae itself learning either format.

- **Build:** the worm claims `.json` and `.toml`, and the pipeline lowers a
  claimed file to `return { ... }` in the output tree.
- **Editor:** the worm's `[lsp]` tier-1 hooks resolve those requires and
  hand the analyzer the same lowering, so the inferred structural type of
  the data shows in hover and completion.

Install by path while developing:

```toml
[worms]
data = { path = "../json-toml-worm" }
```

Build the worm with `cargo build --release`, and point `entry` at the
binary or keep a copy beside `worm.toml`.
