use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use serde_json::Value;

/// Parsed forwarded-args bundle exposed to user pipelines as `$context.args` / `$context.options`.
#[derive(Debug, Clone, Default)]
pub struct ScriptArgs {
    pub args: Vec<String>,
    pub options: HashMap<String, Value>,
}

static GLOBAL_SCRIPT_ARGS: OnceCell<Arc<ScriptArgs>> = OnceCell::new();

/// Install the parsed CLI argv as the process-wide default surfaced on `$context`.
/// Called once at startup from `main`. Idempotent: later calls are no-ops.
pub fn init_global(parsed: ScriptArgs) {
    let _ = GLOBAL_SCRIPT_ARGS.set(Arc::new(parsed));
}

/// Get the process-wide default args/options, or an empty bundle if `init_global` was never called
/// (e.g. in unit tests).
pub fn global() -> Arc<ScriptArgs> {
    GLOBAL_SCRIPT_ARGS
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(ScriptArgs::default()))
}

/// Parse the GNU-ish argv slice forwarded after `--` (clap's `last = true` field).
///
/// Rules (v1, locked):
///   - `--key=value` → `options[key] = "value"` (string)
///   - `--key value`  → `options[key] = "value"` when the next token does not start with `--`
///   - `--key`        → `options[key] = true` when followed by another `--…` or end-of-args
///   - last-wins on repeats
///   - bare positionals (anything not starting with `--`) accumulate into `args`
///   - no short flags, no `--` sentinel inside (clap already stripped the outer one)
pub fn parse(tokens: &[String]) -> ScriptArgs {
    let mut out = ScriptArgs::default();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if let Some(rest) = tok.strip_prefix("--") {
            if rest.is_empty() {
                // A bare `--` inside the forwarded list is just a positional; keep it.
                out.args.push(tok.clone());
                i += 1;
                continue;
            }

            if let Some(eq_idx) = rest.find('=') {
                let (k, v) = rest.split_at(eq_idx);
                out.options.insert(k.to_string(), Value::String(v[1..].to_string()));
                i += 1;
            } else {
                let next_is_flag = tokens
                    .get(i + 1)
                    .map(|n| n.starts_with("--"))
                    .unwrap_or(true);
                if next_is_flag {
                    out.options.insert(rest.to_string(), Value::Bool(true));
                    i += 1;
                } else {
                    out.options
                        .insert(rest.to_string(), Value::String(tokens[i + 1].clone()));
                    i += 2;
                }
            }
        } else {
            out.args.push(tok.clone());
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_str(s: &str) -> ScriptArgs {
        let toks: Vec<String> = s.split_whitespace().map(|t| t.to_string()).collect();
        parse(&toks)
    }

    #[test]
    fn positionals_only() {
        let r = parse_str("Berlin Paris");
        assert_eq!(r.args, vec!["Berlin", "Paris"]);
        assert!(r.options.is_empty());
    }

    #[test]
    fn equals_form() {
        let r = parse_str("--units=imperial");
        assert_eq!(r.options.get("units"), Some(&json!("imperial")));
    }

    #[test]
    fn space_form() {
        let r = parse_str("--units imperial");
        assert_eq!(r.options.get("units"), Some(&json!("imperial")));
    }

    #[test]
    fn boolean_flag_before_other_flag() {
        let r = parse_str("--verbose --units imperial");
        assert_eq!(r.options.get("verbose"), Some(&json!(true)));
        assert_eq!(r.options.get("units"), Some(&json!("imperial")));
    }

    #[test]
    fn boolean_flag_at_end() {
        let r = parse_str("--verbose");
        assert_eq!(r.options.get("verbose"), Some(&json!(true)));
    }

    #[test]
    fn last_wins_on_repeat() {
        let r = parse_str("--units metric --units imperial");
        assert_eq!(r.options.get("units"), Some(&json!("imperial")));
    }

    #[test]
    fn positional_and_flags_interleaved() {
        let r = parse_str("Berlin --units imperial Paris --verbose");
        assert_eq!(r.args, vec!["Berlin", "Paris"]);
        assert_eq!(r.options.get("units"), Some(&json!("imperial")));
        assert_eq!(r.options.get("verbose"), Some(&json!(true)));
    }
}
