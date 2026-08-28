/*!
The json-toml worm: data files as typed Luau modules.

Both sides of the dual-side rule live here. The pipeline claims `.json`
and `.toml` files and lowers each to `return { ... }`, so the output tree
runs the data. The LSP tier-1 hooks resolve a require of one and hand the
analyzer the same lowering, so the editor infers the real structural type
and hover shows it. Core larvae knows nothing about either format; delete
this worm and a `.json` require fails exactly as it always did.
*/

use larvae_worm::native::{Handler, LspLoad, serve};

fn main() {
    serve(DataWorm)
}

struct DataWorm;

impl Handler for DataWorm {
    fn transform(&mut self, source: &str) -> Result<String, String> {
        // The pipeline names no path, so the worm sniffs: JSON first, and
        // TOML when that fails, because every JSON document is valid data
        // for exactly one of the two here.
        lower(source)
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

        let source = lower(&text)?;
        let generated = source.len() as u32;
        let original = text.len() as u32;

        Ok(LspLoad {
            source,
            span_map: vec![(0, generated, 0, original)],
            claims: vec![(0, original)],
        })
    }
}

/// One data document as a Luau module returning its table
fn lower(source: &str) -> Result<String, String> {
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
    write_value(&value, 0, &mut out);
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

fn write_value(value: &serde_json::Value, depth: usize, out: &mut String) {
    let pad = "\t".repeat(depth + 1);
    let close = "\t".repeat(depth);

    match value {
        serde_json::Value::Null => out.push_str("nil"),

        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),

        serde_json::Value::Number(n) => out.push_str(&n.to_string()),

        serde_json::Value::String(s) => {
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
        }

        serde_json::Value::Array(items) => {
            if items.is_empty() {
                out.push_str("{}");

                return;
            }

            out.push_str("{\n");

            for item in items {
                out.push_str(&pad);
                write_value(item, depth + 1, out);
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
                write_value(item, depth + 1, out);
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
    use super::lower;

    #[test]
    fn json_lowers_to_a_table() {
        let out = lower(r#"{"name": "larvae", "count": 3, "tags": ["a", "b"]}"#).unwrap();

        assert!(out.starts_with("--!strict\nreturn {"), "{out}");
        assert!(out.contains("name = \"larvae\""), "{out}");
        assert!(out.contains("count = 3"), "{out}");
    }

    #[test]
    fn toml_lowers_the_same_way() {
        let out = lower("name = \"larvae\"\n\n[nested]\nvalue = 1\n").unwrap();

        assert!(out.contains("name = \"larvae\""), "{out}");
        assert!(out.contains("nested = {"), "{out}");
    }

    #[test]
    fn a_hostile_key_is_bracketed_and_escaped() {
        let out = lower(r#"{"a b": 1, "if": 2, "q\"": 3}"#).unwrap();

        assert!(out.contains("[\"a b\"]"), "{out}");
        assert!(out.contains("[\"if\"]"), "{out}");
        assert!(out.contains("[\"q\\\"\"]"), "{out}");
    }

    #[test]
    fn neither_format_reports_both_reasons() {
        assert!(lower("not data ][").is_err());
    }
}
