//! The `credential` and `mcp` bindings (policy-1 `bindings[]`), as bwrap.py carries them out: `credentials` (the
//! values, read on the host at run time from each binding's `from`), `secret_names` and `deliver` (a `via: file`
//! value as a file of the run's, its variable that file's path) and `mcp_servers` (the run's `/run/pot/mcp.json`).
//!
//! A binding holds references only (`from`: a key file or a host variable's name); the values exist in `run`'s
//! memory, on the named pipe and, for `via: file`, in the run folder's `secrets/NAME` (policy-1 rule 6's one file).

use std::collections::BTreeMap;
use std::ffi::OsString;

use serde::{Deserialize, Serialize};

use crate::policy::Policy;

/// What `run` needs of a `credential` or `mcp` binding: references, never a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cred {
    pub capability: String,
    pub selector: String,
    pub env: Vec<String>,
    pub from: serde_json::Value,
    pub via: String,
}

/// The run's credential and mcp bindings, in bwrap's order: the credential bindings, then the mcp ones.
pub fn creds(p: &Policy) -> Vec<Cred> {
    ["credential", "mcp"]
        .iter()
        .flat_map(|cap| p.bindings.iter().filter(move |b| b.capability == *cap))
        .map(|b| Cred {
            capability: b.capability.clone(),
            selector: b.selector.clone(),
            env: b.env.clone().unwrap_or_default(),
            from: b.from.clone().unwrap_or(serde_json::Value::Null),
            via: b.via.clone().unwrap_or_default(),
        })
        .collect()
}

/// Every binding's variables, each once, in order.
pub fn names(c: &[Cred]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for n in c.iter().flat_map(|b| b.env.iter()) {
        if !out.contains(n) {
            out.push(n.clone());
        }
    }
    out
}

/// bwrap's `secret_names`: the variables of a binding with `via: file`, each with that binding (the last one that
/// names it).
pub fn via_file(c: &[Cred]) -> BTreeMap<String, &Cred> {
    c.iter()
        .filter(|b| b.via == "file")
        .flat_map(|b| b.env.iter().map(move |n| (n.clone(), b)))
        .collect()
}

/// One value `credentials` read: the variable, its value, and the binding it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Val {
    pub name: String,
    pub value: OsString,
    pub capability: String,
    pub selector: String,
}

/// bwrap's `key_file`: `NAME=VALUE` lines (`export NAME=VALUE` too; `#` comments and blank lines skipped; one pair
/// of matching quotes stripped), or None when it holds no such line. Later lines win.
pub fn key_file(text: &str) -> Option<BTreeMap<String, String>> {
    let mut vals = BTreeMap::new();
    for line in universal(text).split('\n') {
        let line = py_strip(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = match line.strip_prefix("export ") {
            Some(rest) => rest.trim_start_matches(py_space),
            None => line,
        };
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = py_strip(k);
        if !is_name(k) {
            continue;
        }
        let mut v = py_strip(v);
        let b = v.as_bytes();
        if b.len() >= 2 && b[0] == b[b.len() - 1] && (b[0] == b'"' || b[0] == b'\'') {
            v = &v[1..v.len() - 1];
        }
        vals.insert(k.to_string(), v.to_string());
    }
    (!vals.is_empty()).then_some(vals)
}

/// Python's text mode: `\r\n` and `\r` read as `\n` (universal newlines).
fn universal(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Python's `str.isspace`, for `str.strip`.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

fn py_strip(s: &str) -> &str {
    s.trim_matches(py_space)
}

/// `[A-Za-z_][A-Za-z0-9_]*`: a variable's name.
pub fn is_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// Python's `os.path.expanduser` for `~` and `~/…` (HOME, as the host has it).
fn expand_home(f: &str, getenv: &dyn Fn(&str) -> Option<OsString>) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt as _;
    match (f.strip_prefix('~'), getenv("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            // posixpath: HOME without its trailing '/', then the rest; '/' when that is empty
            let home = home.as_bytes();
            let end = home.iter().rposition(|&b| b != b'/').map_or(0, |i| i + 1);
            let mut p = home[..end].to_vec();
            p.extend_from_slice(rest.as_bytes());
            if p.is_empty() {
                p.push(b'/');
            }
            std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&p))
        }
        _ => std::path::PathBuf::from(f),
    }
}

/// An I/O error as Python's `strerror` says it: without Rust's ` (os error N)`.
fn strerror(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_string(),
        _ => s,
    }
}

/// bwrap's `credentials`: the values the bindings pass, read on the host now (at `run`), never written to a run file,
/// a log or an argument. `from.file` or `from.envFile`: a key file of `NAME=VALUE` lines naming the variables; a
/// credential's file with no such line is one value, the whole file (a token file). `from.env` (a host variable, for
/// a binding of one variable) or nothing: the host's variables of the same names. A variable the host has unset is
/// left out. GETENV reads the host's variables (`HOME` too, for a key file's `~`).
pub fn read(c: &[Cred], getenv: &dyn Fn(&str) -> Option<OsString>) -> Result<Vec<Val>, String> {
    let mut out: Vec<Val> = vec![];
    let mut put = |b: &Cred, name: &str, value: OsString| {
        let v = Val {
            name: name.to_string(),
            value,
            capability: b.capability.clone(),
            selector: b.selector.clone(),
        };
        match out.iter_mut().find(|x| x.name == name) {
            Some(x) => *x = v,
            None => out.push(v),
        }
    };
    for b in c {
        let src = |k: &str| b.from.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        let (cap, sel, names) = (&b.capability, &b.selector, &b.env);
        if let Some(f) = src("file").or_else(|| src("envFile")) {
            let path = expand_home(f, getenv);
            let text = std::fs::read(&path)
                .map_err(|e| format!("the {cap} binding {sel} reads {f}: {}", strerror(&e)))
                .and_then(|b| {
                    String::from_utf8(b)
                        .map_err(|_| format!("the {cap} binding {sel} reads {f}: it is not UTF-8 text"))
                })?;
            let vals = if let Some(v) = key_file(&text) {
                v
            } else {
                if cap == "mcp" || names.len() != 1 || src("envFile").is_some() {
                    return Err(format!(
                        "the {cap} binding {sel} reads {f}, which holds no NAME=VALUE line for {}: write {}=… in it",
                        if names.is_empty() {
                            "its variable".to_string()
                        } else {
                            names.join(", ")
                        },
                        names.first().map_or("NAME", String::as_str)
                    ));
                }
                [(names[0].clone(), py_strip(&universal(&text)).to_string())]
                    .into_iter()
                    .collect()
            };
            let missing: Vec<&str> = names
                .iter()
                .filter(|n| !vals.contains_key(*n) && *n != "AWS_CREDENTIAL_EXPIRATION")
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "the {cap} binding {sel} reads {f}, which has no {}",
                    missing.join(", ")
                ));
            }
            for n in names {
                if let Some(v) = vals.get(n) {
                    put(b, n, v.into());
                }
            }
        } else {
            let host = b
                .from
                .get("env")
                .and_then(|v| v.as_str())
                .filter(|_| names.len() == 1);
            for n in names {
                if let Some(v) = getenv(host.unwrap_or(n)) {
                    put(b, n, v);
                }
            }
        }
    }
    Ok(out)
}

/// bwrap's `deliver`'s refusals, before anything is written: an mcp binding's `via: file` key with a quote, a
/// backslash or a control character, which its header helper can't carry; and a NUL in a value that goes as a
/// variable, which no variable can hold.
pub fn check(c: &[Cred], vals: &[Val]) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt as _;
    let files = via_file(c);
    for v in vals {
        let bytes = v.value.as_bytes();
        match files.get(&v.name) {
            Some(b) if b.capability == "mcp" => {
                if bytes
                    .iter()
                    .any(|&x| x == b'"' || x == b'\\' || x < 0x20 || x == 0x7f)
                {
                    return Err(format!(
                        "the mcp binding {}: its key holds a quote, a backslash or a control character, which its header helper cannot carry: use via = \"env\"",
                        b.selector
                    ));
                }
            }
            None if bytes.contains(&0) => {
                return Err(format!(
                    "the {} binding {}: the value of {} holds a NUL, which a variable can't carry",
                    v.capability, v.selector, v.name
                ));
            }
            Some(_) | None => {}
        }
    }
    Ok(())
}

/// bwrap's `mcp_servers`, written as `json.dump(…, indent=2)` writes it: the run's `/run/pot/mcp.json`. The key is
/// never in it: with `via: env` the header names its variable (`${NAME}`, which the client expands); with `via: file`
/// a `headersHelper` reads the key from the file the variable names, with shell builtins, and prints the header.
pub fn mcp_json(p: &Policy) -> String {
    let mut servers: Vec<(String, J)> = vec![];
    for m in p.bindings.iter().filter(|b| b.capability == "mcp") {
        let mut srv = vec![
            ("type".to_string(), J::Str("http".into())),
            ("url".to_string(), J::Str(m.url.clone().unwrap_or_default())),
        ];
        if let Some(env) = m.env.as_ref().and_then(|e| e.first()) {
            let scheme = m.scheme.as_deref().unwrap_or("Bearer");
            let header = m
                .header
                .as_deref()
                .filter(|h| !h.is_empty())
                .unwrap_or("Authorization");
            if m.via.as_deref() == Some("file") {
                let prefix = if scheme.is_empty() {
                    String::new()
                } else {
                    format!("{scheme} ")
                };
                let whole = format!(
                    "{{{}: {}}}",
                    py_json_str(header),
                    py_json_str(&format!("{prefix}\0"))
                );
                let at = whole.rfind("\\u0000").unwrap_or(whole.len());
                let (left, right) = (&whole[..at], &whole[(at + 6).min(whole.len())..]);
                srv.push((
                    "headersHelper".into(),
                    J::Str(format!(
                        "read -r v < \"${env}\"; printf '%s%s%s' {} \"$v\" {}",
                        shlex_quote(left),
                        shlex_quote(right)
                    )),
                ));
            } else {
                let value = if scheme.is_empty() {
                    format!("${{{env}}}")
                } else {
                    format!("{scheme} ${{{env}}}")
                };
                srv.push((
                    "headers".into(),
                    J::Obj(vec![(header.to_string(), J::Str(value))]),
                ));
            }
        }
        let name = m.server.clone().unwrap_or_default();
        match servers.iter_mut().find(|(k, _)| *k == name) {
            Some(s) => s.1 = J::Obj(srv),
            None => servers.push((name, J::Obj(srv))),
        }
    }
    let mut out = String::new();
    J::Obj(vec![("mcpServers".into(), J::Obj(servers))]).dump(&mut out, 0);
    out
}

/// A JSON value as bwrap's `mcp.json` holds them: strings and objects, in order.
enum J {
    Str(String),
    Obj(Vec<(String, J)>),
}

impl J {
    /// Python's `json.dump(…, indent=2)`.
    fn dump(&self, out: &mut String, level: usize) {
        match self {
            J::Str(s) => out.push_str(&py_json_str(s)),
            J::Obj(items) if items.is_empty() => out.push_str("{}"),
            J::Obj(items) => {
                out.push_str("{\n");
                for (i, (k, v)) in items.iter().enumerate() {
                    out.push_str(&"  ".repeat(level + 1));
                    out.push_str(&py_json_str(k));
                    out.push_str(": ");
                    v.dump(out, level + 1);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                out.push_str(&"  ".repeat(level));
                out.push('}');
            }
        }
    }
}

/// A JSON string as Python's `json.dumps` writes it (`ensure_ascii`: everything outside ' '..'~' escaped).
fn py_json_str(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{u:04x}");
                }
            }
        }
    }
    out.push('"');
    out
}

/// Python's `shlex.quote`.
fn shlex_quote(s: &str) -> String {
    if s.is_empty() {
        "''".into()
    } else if s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cred(cap: &str, sel: &str, env: &[&str], from: serde_json::Value, via: &str) -> Cred {
        Cred {
            capability: cap.into(),
            selector: sel.into(),
            env: env.iter().map(|s| (*s).to_string()).collect(),
            from,
            via: via.into(),
        }
    }

    /// A folder of fake key files for one test.
    fn keys(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pot-container-unit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn vals(v: &[Val]) -> Vec<(String, String)> {
        v.iter()
            .map(|x| (x.name.clone(), x.value.to_string_lossy().into_owned()))
            .collect()
    }

    #[test]
    fn a_key_file_is_bwraps_name_value_lines() {
        let got =
            key_file("# a comment\n\nexport A=1\nB = \"two\" \n C='3'\nD=\"x'\nnot a pair\n9X=no\nA=last\n")
                .unwrap();
        let want: BTreeMap<String, String> = [("A", "last"), ("B", "two"), ("C", "3"), ("D", "\"x'")]
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        assert_eq!(got, want);
        assert_eq!(key_file("just-a-token\n"), None);
        assert_eq!(key_file("\r\n# only a comment\r\n"), None);
    }

    #[test]
    fn each_from_kind_is_read_as_bwrap_reads_it() {
        let d = keys("from");
        std::fs::write(d.join("token"), "  tok-value\n").unwrap();
        std::fs::write(d.join("pair.env"), "export B_KEY=b-value\r\nOTHER=x\r\n").unwrap();
        std::fs::write(d.join("home.env"), "C_KEY=c-value\n").unwrap();
        let c = vec![
            cred(
                "credential",
                "a",
                &["A_TOK"],
                serde_json::json!({ "file": d.join("token") }),
                "file",
            ),
            cred(
                "credential",
                "b",
                &["B_KEY"],
                serde_json::json!({ "envFile": d.join("pair.env") }),
                "env",
            ),
            cred(
                "credential",
                "c",
                &["C_KEY"],
                serde_json::json!({ "file": "~/home.env" }),
                "env",
            ),
            cred(
                "credential",
                "d",
                &["D_KEY"],
                serde_json::json!({ "env": "HOST_D" }),
                "env",
            ),
            cred(
                "credential",
                "e",
                &["E_KEY", "E_UNSET"],
                serde_json::json!({}),
                "env",
            ),
        ];
        let home = d.display().to_string();
        let env = move |n: &str| match n {
            "HOME" => Some(OsString::from(&home)),
            "HOST_D" => Some("d-value".into()),
            "E_KEY" => Some("e-value".into()),
            "D_KEY" => Some("not-this".into()),
            _ => None,
        };
        let got = read(&c, &env).unwrap();
        assert_eq!(
            vals(&got),
            [
                ("A_TOK", "tok-value"),
                ("B_KEY", "b-value"),
                ("C_KEY", "c-value"),
                ("D_KEY", "d-value"),
                ("E_KEY", "e-value"),
            ]
            .map(|(a, b)| (a.to_string(), b.to_string()))
        );
        assert_eq!(got[0].selector, "a");
        assert_eq!(
            names(&c),
            ["A_TOK", "B_KEY", "C_KEY", "D_KEY", "E_KEY", "E_UNSET"]
        );
        let files: Vec<String> = via_file(&c).into_keys().collect();
        assert_eq!(files, ["A_TOK"]);
    }

    #[test]
    fn a_key_files_errors_name_the_binding_and_file_never_the_value() {
        let d = keys("errors");
        std::fs::write(d.join("token"), "secret-token-value\n").unwrap();
        std::fs::write(d.join("pair.env"), "OTHER=secret-other-value\n").unwrap();
        let none = |_: &str| None;
        let f = |p: &str| d.join(p).display().to_string();
        let cases = [
            (
                cred(
                    "mcp",
                    "s",
                    &["MCPKEY"],
                    serde_json::json!({ "file": f("token") }),
                    "env",
                ),
                format!(
                    "the mcp binding s reads {}, which holds no NAME=VALUE line for MCPKEY: write MCPKEY=… in it",
                    f("token")
                ),
            ),
            (
                cred(
                    "credential",
                    "two",
                    &["A", "B"],
                    serde_json::json!({ "file": f("token") }),
                    "env",
                ),
                format!(
                    "the credential binding two reads {}, which holds no NAME=VALUE line for A, B: write A=… in it",
                    f("token")
                ),
            ),
            (
                cred(
                    "credential",
                    "ef",
                    &["A"],
                    serde_json::json!({ "envFile": f("token") }),
                    "env",
                ),
                format!(
                    "the credential binding ef reads {}, which holds no NAME=VALUE line for A: write A=… in it",
                    f("token")
                ),
            ),
            (
                cred(
                    "credential",
                    "miss",
                    &["A", "AWS_CREDENTIAL_EXPIRATION"],
                    serde_json::json!({ "file": f("pair.env") }),
                    "env",
                ),
                format!(
                    "the credential binding miss reads {}, which has no A",
                    f("pair.env")
                ),
            ),
            (
                cred(
                    "credential",
                    "gone",
                    &["A"],
                    serde_json::json!({ "file": f("nothing") }),
                    "file",
                ),
                format!(
                    "the credential binding gone reads {}: No such file or directory",
                    f("nothing")
                ),
            ),
        ];
        for (c, want) in cases {
            let got = read(&[c], &none).unwrap_err();
            assert_eq!(got, want);
            assert!(!got.contains("secret-"), "{got}");
        }
        // AWS_CREDENTIAL_EXPIRATION may be missing
        let c = cred(
            "credential",
            "aws",
            &["OTHER", "AWS_CREDENTIAL_EXPIRATION"],
            serde_json::json!({ "file": f("pair.env") }),
            "env",
        );
        assert_eq!(
            vals(&read(&[c], &none).unwrap()),
            [("OTHER".to_string(), "secret-other-value".to_string())]
        );
    }

    #[test]
    fn an_mcp_via_file_key_with_a_quote_backslash_or_control_is_refused() {
        let c = vec![
            cred("mcp", "s", &["MCPKEY"], serde_json::json!({}), "file"),
            cred("credential", "t", &["TOK"], serde_json::json!({}), "env"),
        ];
        let v = |n: &str, s: &str| Val {
            name: n.into(),
            value: s.into(),
            capability: if n == "MCPKEY" {
                "mcp".into()
            } else {
                "credential".into()
            },
            selector: if n == "MCPKEY" { "s".into() } else { "t".into() },
        };
        assert_eq!(
            check(&c, &[v("MCPKEY", "plain-key"), v("TOK", "a\"b\\c")]),
            Ok(())
        );
        for bad in ["a\"b", "a\\b", "a\nb", "a\x7fb", "a\tb"] {
            let got = check(&c, &[v("MCPKEY", bad)]).unwrap_err();
            assert_eq!(
                got,
                "the mcp binding s: its key holds a quote, a backslash or a control character, which its header helper cannot carry: use via = \"env\""
            );
        }
        assert_eq!(
            check(&c, &[v("TOK", "a\0b")]).unwrap_err(),
            "the credential binding t: the value of TOK holds a NUL, which a variable can't carry"
        );
    }

    fn mcp_policy(via: &str, scheme: &str, header: &str) -> Policy {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../contracts/fixtures/policy-1/work-bindings.json"),
        )
        .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let b = &mut v["bindings"][2];
        b["via"] = via.into();
        b["scheme"] = scheme.into();
        b["header"] = header.into();
        crate::policy::parse(&v.to_string()).unwrap()
    }

    #[test]
    fn mcp_json_is_bwraps_and_never_holds_a_value() {
        assert_eq!(
            mcp_json(&mcp_policy("env", "Bearer", "Authorization")),
            "{\n  \"mcpServers\": {\n    \"s\": {\n      \"type\": \"http\",\n      \"url\": \"https://mcp.example/mcp\",\n      \"headers\": {\n        \"Authorization\": \"Bearer ${MCPKEY}\"\n      }\n    }\n  }\n}"
        );
        assert!(mcp_json(&mcp_policy("env", "", "X-Key")).contains("\"X-Key\": \"${MCPKEY}\""));
        assert_eq!(
            mcp_json(&mcp_policy("file", "Bearer", "Authorization")),
            "{\n  \"mcpServers\": {\n    \"s\": {\n      \"type\": \"http\",\n      \"url\": \"https://mcp.example/mcp\",\n      \"headersHelper\": \"read -r v < \\\"$MCPKEY\\\"; printf '%s%s%s' '{\\\"Authorization\\\": \\\"Bearer ' \\\"$v\\\" '\\\"}'\"\n    }\n  }\n}"
        );
        // no scheme, and a header name JSON escapes, as Python's json.dumps does (ensure_ascii)
        let helper = mcp_json(&mcp_policy("file", "", "X-Clé"));
        assert!(
            helper.contains(r#"printf '%s%s%s' '{\"X-Cl\\u00e9\": \"' \"$v\" '\"}'"#),
            "{helper}"
        );
        // no mcp binding: bwrap's empty configuration
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/reference/container/prepare.in/read-only/policy.json"),
        )
        .unwrap();
        assert_eq!(
            mcp_json(&crate::policy::parse(&text).unwrap()),
            "{\n  \"mcpServers\": {}\n}"
        );
    }

    #[test]
    fn the_policys_bindings_in_bwraps_order() {
        let p = mcp_policy("file", "Bearer", "Authorization");
        let c = creds(&p);
        assert_eq!(
            c.iter()
                .map(|c| (c.capability.as_str(), c.via.as_str()))
                .collect::<Vec<_>>(),
            [("credential", "file"), ("mcp", "file")]
        );
        assert_eq!(c[0].from, serde_json::json!({ "file": "/tmp/pot-test/sbtok" }));
        assert_eq!(
            via_file(&c).into_keys().collect::<Vec<_>>(),
            ["MCPKEY", "TESTTOK"]
        );
    }
}
