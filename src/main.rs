/*!
The json-toml worm: data files as Luau modules.

The pipeline claims `.json` and `.toml` files and lowers each to
`return { ... }`, so the output tree runs the data. Core larvae knows
nothing about either format; delete this worm and a `.json` require fails
exactly as it always did.
*/

use larvae_worm::native::{serve, Handler};

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
    Strict mode, so a type checker that reads the output infers the
    literal types: { name: string, port: number }, not a table of any.
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

fn write_value(value: &serde_json::Value, depth: usize, out: &mut String) {
    let pad = "\t".repeat(depth + 1);
    let close = "\t".repeat(depth);

    match value {
        serde_json::Value::Null => out.push_str("nil"),

        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),

        serde_json::Value::Number(n) => out.push_str(&n.to_string()),

        serde_json::Value::String(s) => out.push_str(&quoted(s)),

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
