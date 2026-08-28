/*!
The json-toml worm: data files as typed Luau modules.

Both sides of the dual-side rule live here. The pipeline claims `.json`
and `.toml` files and lowers each to `return { ... }`, so the output tree
runs the data. The LSP tier-1 hooks resolve a require of one and hand the
analyzer the same lowering, so the editor infers the real structural type
and hover shows it. Core larvae knows nothing about either format; delete
this worm and a `.json` require fails exactly as it always did.
*/

use larvae_worm::native::{Handler, LspLoad, Settings, serve};

fn main() {
    serve(DataWorm::default())
}

#[derive(Default)]
struct DataWorm {
    /*
    `[worms.data.config] force_string_singletons`, parsed once at init.

    A file glob names which data files it covers, and each value glob
    names which string values narrow: a matched string lowers as
    `"kind" :: "kind"`, so the analyzer reads the literal and a type
    function can switch on it. luau-lsp carries the same knob as
    `types.dataFileForceStringletons`.
    */
    singletons: Vec<(String, Vec<String>)>,
}

impl Handler for DataWorm {
    fn init(&mut self, config: &str, _rules: &str, _settings: &Settings) -> Result<(), String> {
        // The host sends the config table as text; either spelling parses.
        let parsed: serde_json::Value = serde_json::from_str(config)
            .or_else(|_| {
                config
                    .parse::<toml::Table>()
                    .map_err(|e| e.to_string())
                    .and_then(|t| serde_json::to_value(t).map_err(|e| e.to_string()))
            })
            .unwrap_or_default();

        if let Some(map) = parsed["force_string_singletons"].as_object() {
            self.singletons = map
                .iter()
                .map(|(file, values)| {
                    let globs = values
                        .as_array()
                        .map(|list| {
                            list.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();

                    (file.clone(), globs)
                })
                .collect();
        }

        Ok(())
    }

    fn transform(&mut self, source: &str) -> Result<String, String> {
        // The pipeline names no path, so the worm sniffs: JSON first, and
        // TOML when that fails, because every JSON document is valid data
        // for exactly one of the two here.
        lower(source, &[])
    }

    fn lsp_resolve(&mut self, from: &str, spec: &str) -> Result<Option<String>, String> {
        // The hook joins against the requiring file, so an alias is not ours.
        if spec.starts_with('@') {
            return Ok(None);
        }

        let base = std::path::Path::new(from)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));

        /*
        The extension is optional, because the build side drops it: larvae
        names a claimed `test.json` after its stem, so `require("./test")`
        is what the author writes and what runs in the place. The editor
        answers both spellings, or the two sides disagree and the hover
        falls back to any.
        */
        let candidates = match spec.ends_with(".json") || spec.ends_with(".toml") {
            true => vec![spec.to_string()],

            false => vec![format!("{spec}.json"), format!("{spec}.toml")],
        };

        for candidate in candidates {
            if let Ok(path) = base.join(&candidate).canonicalize() {
                return Ok(Some(path.to_string_lossy().into_owned()));
            }
        }

        Ok(None)
    }

    /*
    Inside a data file, the analyzer has nothing to offer.

    The lowering is a Luau module, so the analyzer would answer a ctrl-space
    in a `.toml` with the Luau global scope, which is noise in a file that
    holds no code. The list empties for the claimed formats and no other:
    a completion in a plain Luau file passes through untouched, whoever
    asked for it.
    */
    fn lsp_respond(
        &mut self,
        kind: &str,
        context: &str,
        _response: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        if kind != "completions" {
            return Ok(None);
        }

        let context: serde_json::Value = serde_json::from_str(context).unwrap_or_default();
        let claimed = context["path"]
            .as_str()
            .is_some_and(|p| p.ends_with(".json") || p.ends_with(".toml"));

        Ok(claimed.then(|| serde_json::json!([])))
    }

    fn lsp_load(&mut self, path: &str) -> Result<LspLoad, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;

        let normalized = path.replace('\\', "/");
        let value_globs: Vec<String> = self
            .singletons
            .iter()
            .filter(|(file, _)| glob_match(file, &normalized))
            .flat_map(|(_, values)| values.iter().cloned())
            .collect();

        let source = lower(&text, &value_globs)?;
        let generated = source.len() as u32;
        let original = text.len() as u32;

        Ok(LspLoad {
            source,
            span_map: vec![(0, generated, 0, original)],
            claims: vec![(0, original)],
        })
    }
}

/*
Whether a glob covers a text.

`**` crosses `/`, `*` stops at one, `?` is one character. Written here
because the worm's one dependency is serde_json, and thirty lines do not
earn another.
*/
fn glob_match(pattern: &str, text: &str) -> bool {
    fn inner(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,

            (Some(b'*'), _) if p.get(1) == Some(&b'*') => {
                // `**` eats any run, separators included.
                inner(&p[2..], t) || (!t.is_empty() && inner(p, &t[1..]))
            }

            (Some(b'*'), _) => {
                inner(&p[1..], t)
                    || (!t.is_empty() && t[0] != b'/' && inner(p, &t[1..]))
            }

            (Some(b'?'), Some(_)) => inner(&p[1..], &t[1..]),

            (Some(a), Some(b)) if a == b => inner(&p[1..], &t[1..]),

            _ => false,
        }
    }

    // A path glob with no directory part matches by the file name alone.
    if !pattern.contains('/') {
        let name = text.rsplit('/').next().unwrap_or(text);

        return inner(pattern.as_bytes(), name.as_bytes());
    }

    inner(pattern.as_bytes(), text.as_bytes())
}

/// One data document as a Luau module returning its table
fn lower(source: &str, singletons: &[String]) -> Result<String, String> {
    let value = match serde_json::from_str::<serde_json::Value>(source) {
        Ok(v) => v,

        Err(_) => {
            /*
            A document parses as a table, never as a bare value: toml 0.9
            reads `Value::from_str` as one value expression, and a file is
            a document.
            */
            let parsed: toml::Table = source
                .parse()
                .map_err(|e| format!("neither JSON nor TOML: {e}"))?;

            toml_to_json(toml::Value::Table(parsed))
        }
    };

    /*
    Strict mode, so the analyzer infers the literal types precisely: a
    hover shows { name: string, port: number }, not a table of any.
    */
    let mut out = String::from("--!strict
return ");
    write_value(&value, 0, &mut out, singletons);
    out.push('\n');

    Ok(out)
}

fn toml_to_json(value: toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s),

        toml::Value::Integer(i) => serde_json::json!(i),

        toml::Value::Float(f) => serde_json::json!(f),

        toml::Value::Boolean(b) => serde_json::Value::Bool(b),

        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),

        toml::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(toml_to_json).collect())
        }

        toml::Value::Table(table) => serde_json::Value::Object(
            table
                .into_iter()
                .map(|(k, v)| (k, toml_to_json(v)))
                .collect(),
        ),
    }
}

/// A Luau string literal for one value, escaped.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');

    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{}", c as u32)),
            c => out.push(c),
        }
    }

    out.push('"');
    out
}

fn write_value(value: &serde_json::Value, depth: usize, out: &mut String, singletons: &[String]) {
    let pad = "\t".repeat(depth + 1);
    let close = "\t".repeat(depth);

    match value {
        serde_json::Value::Null => out.push_str("nil"),

        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),

        serde_json::Value::Number(n) => out.push_str(&n.to_string()),

        serde_json::Value::String(s) => {
            let literal = quoted(s);
            out.push_str(&literal);

            /*
            The narrowed spelling, where the project asked for it: the
            cast keeps the literal as the type, so a hover reads `"kind"`
            and a type function can switch on it. Luau widens a plain
            table field to `string` otherwise.
            */
            if singletons.iter().any(|glob| glob_match(glob, s)) {
                out.push_str(" :: ");
                out.push_str(&literal);
            }
        }

        serde_json::Value::Array(items) => {
            if items.is_empty() {
                out.push_str("{}");

                return;
            }

            out.push_str("{\n");

            for item in items {
                out.push_str(&pad);
                write_value(item, depth + 1, out, singletons);
                out.push_str(",\n");
            }

            out.push_str(&close);
            out.push('}');
        }

        serde_json::Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");

                return;
            }

            out.push_str("{\n");

            for (key, item) in map {
                out.push_str(&pad);

                if is_luau_name(key) {
                    out.push_str(key);
                } else {
                    out.push_str("[\"");
                    out.push_str(&key.replace('\\', "\\\\").replace('"', "\\\""));
                    out.push_str("\"]");
                }

                out.push_str(" = ");
                write_value(item, depth + 1, out, singletons);
                out.push_str(",\n");
            }

            out.push_str(&close);
            out.push('}');
        }
    }
}

fn is_luau_name(key: &str) -> bool {
    !key.is_empty()
        && !key.chars().next().unwrap().is_ascii_digit()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(
            key,
            "and" | "break" | "do" | "else" | "elseif" | "end" | "false" | "for" | "function"
                | "if" | "in" | "local" | "nil" | "not" | "or" | "repeat" | "return" | "then"
                | "true" | "until" | "while"
        )
}

#[cfg(test)]
mod tests {
    use super::{glob_match, lower};

    #[test]
    fn json_lowers_to_a_table() {
        let out = lower(r#"{"name": "larvae", "count": 3, "tags": ["a", "b"]}"#, &[]).unwrap();

        assert!(out.starts_with("--!strict\nreturn {"), "{out}");
        assert!(out.contains("name = \"larvae\""), "{out}");
        assert!(out.contains("count = 3"), "{out}");
    }

    #[test]
    fn toml_lowers_the_same_way() {
        let out = lower("name = \"larvae\"\n\n[nested]\nvalue = 1\n", &[]).unwrap();

        assert!(out.contains("name = \"larvae\""), "{out}");
        assert!(out.contains("nested = {"), "{out}");
    }

    #[test]
    fn a_hostile_key_is_bracketed_and_escaped() {
        let out = lower(r#"{"a b": 1, "if": 2, "q\"": 3}"#, &[]).unwrap();

        assert!(out.contains("[\"a b\"]"), "{out}");
        assert!(out.contains("[\"if\"]"), "{out}");
        assert!(out.contains("[\"q\\\"\"]"), "{out}");
    }

    #[test]
    fn neither_format_reports_both_reasons() {
        assert!(lower("not data ][", &[]).is_err());
    }


    #[test]
    fn a_matched_string_narrows_to_its_literal() {
        let out = lower(r#"{"kind": "melee", "name": "sword"}"#, &["melee".into()]).unwrap();

        assert!(out.contains(r#"kind = "melee" :: "melee""#), "{out}");
        assert!(out.contains(r#"name = "sword","#), "{out}");
        assert!(!out.contains(r#""sword" :: "#), "{out}");
    }

    #[test]
    fn the_glob_speaks_stars_and_separators() {
        assert!(glob_match("item_*", "item_sword"));
        assert!(!glob_match("item_*", "sworditem_"));
        assert!(glob_match("*.json", "/a/b/data.json"));
        assert!(glob_match("**/data/*.json", "/proj/src/data/items.json"));
        assert!(!glob_match("src/*.json", "src/deep/items.json"));
        assert!(glob_match("src/**/*.json", "src/deep/items.json"));
    }
}
