# json-toml-worm

A larvae worm that makes `require("./thing.json")` and
`require("./config.toml")` work in the build, without larvae itself
learning either format. The worm claims `.json` and `.toml`, and the
pipeline lowers a claimed file to `return { ... }` in the output tree.

The editor types are gone. larvae 0.10 removed its type analyzer, and
luau-lsp serves the types now. luau-lsp does not read a data file, so a
require of one has no structural type in the editor. A luau-lsp plugin can
bring the types back for JSON: a `transformSource` script can read the file
with `lsp.fs.readFile` and parse it with `lsp.json.deserialize`. That
plugin does not exist yet.

Install by path while developing:

```toml
[worms]
data = { path = "../json-toml-worm" }
```

Build the worm with `cargo build --release`, and point `entry` at the
binary or keep a copy beside `worm.toml`.
