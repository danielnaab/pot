//! policy.json/1 (contracts/policy-1.md), read as a backend built apart from pot must: strictly.
//!
//! Every object pot builds is closed, so an unknown field is refused (a field this backend does not know could
//! limit the run, and skipping it would run it wider). The objects the contract keeps open (a binding's `from`
//! and `key`) and the ones that are Info (sources, a capability item's `data`) are kept as JSON values.
//!
//! [`parse`] checks, in order: the version (refusal rule 1), the shape (unknown fields, types), then the rules
//! the schema states that a backend relies on (the conditionals, value patterns, known kinds and capabilities:
//! rules 3 and 4). The backend's name (rule 1) and its guarantees (rule 2) are checked by [`check_backend`].

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// A refusal: one `pot (<context>): …` line on stderr, exit 1.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub context: String,
    pub message: String,
}

impl Refusal {
    pub fn new(context: &str, message: impl Into<String>) -> Self {
        Refusal {
            context: context.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pot ({}): {}", self.context, self.message)
    }
}

/// This backend's name in `backend.name`.
pub const BACKEND: &str = "container";

/// The guarantee names policy.json/1 knows (the schema's `guarantee` enum).
pub const GUARANTEES: &[&str] = &[
    "closure-store",
    "own-netns",
    "own-pidns",
    "no-tiocsti",
    "protected-as-mounts",
    "local-listen",
    "resource-limits",
    "separate-kernel",
    "in-place",
    "same-path-clone",
    "nest",
    "private-proxy",
    "root-files",
];

/// Capabilities a binding may name in policy.json/1, and the reserved ones compose never binds.
pub const CAPABILITIES: &[&str] = &["status.relay", "git.fetch", "credential", "mcp", "nix"];
pub const RESERVED: &[&str] = &["git.push", "git.sign", "ops.read", "delegate"];

/// Item kinds and the class each belongs to (the schema's conditionals). A kind not listed is unknown.
const KINDS: &[(&str, &str)] = &[
    ("package", "see"),
    ("env.set", "see"),
    ("file", "see"),
    ("dir", "see"),
    ("access", "limits"),
    ("hidden", "limits"),
    ("protect", "limits"),
    ("network", ""), // limits (value none) or reach (host:port)
    ("include", "reach"),
    ("view", "reach"),
    ("read", "reach"),
    ("write", "reach"),
    ("socket", "reach"),
    ("env", "reach"),
    ("state", "reach"),
    ("capability", "reach"),
    ("unsandboxed", "reach"),
];

pub fn known_kind(kind: &str) -> bool {
    KINDS.iter().any(|(k, _)| *k == kind)
}

// Info fields are parsed so that an unknown one is refused; a backend must not act on them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u64,
    pub run: String,
    pub context: String,
    pub mode: String,
    pub access: String,
    pub backend: Backend,
    pub workspace: Vec<Workspace>,
    pub items: Vec<Item>,
    pub mounts: Mounts,
    pub bindings: Vec<Binding>,
    pub children: BTreeMap<String, Child>,
    pub grants: Vec<Grant>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub name: String,
    pub requires: Vec<String>,
    pub proved: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    pub path: String,
    pub common: String,
    pub protected: Vec<String>,
    pub sealed: Vec<String>,
    pub proposal: Vec<String>,
    pub clone: Option<String>,
}

// Info fields are parsed so that an unknown one is refused; a backend must not act on them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Item {
    pub kind: String,
    pub value: String,
    pub class: String,
    pub sources: Vec<Value>,
    pub effective: bool,
    pub why: Option<String>,
    pub request: Option<String>,
    pub request_class: Option<String>,
    pub path: Option<String>,
    pub paths: Option<Vec<String>>,
    pub secret: Option<bool>,
    pub at: Option<String>,
    pub shared: Option<bool>,
    pub data: Option<Value>,
    pub binding: Option<String>,
    pub via: Option<String>,
    pub tools: Option<bool>,
    pub attr: Option<String>,
    pub source: Option<String>,
    pub mode: Option<String>,
    pub keys: Option<Vec<String>>,
    pub memory: Option<bool>,
    pub mirror: Option<String>,
    pub refresh: Option<bool>,
    pub links: Option<Vec<Vec<String>>>,
    pub skipped: Option<Vec<String>>,
    pub home: Option<Vec<String>>,
    pub home_manager: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mounts {
    pub state: Vec<StateMount>,
    pub run: String,
    pub launcher: String,
}

// Info fields are parsed so that an unknown one is refused; a backend must not act on them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateMount {
    pub name: String,
    pub at: String,
    pub key: Vec<String>,
    pub dir: String,
}

// Info fields are parsed so that an unknown one is refused; a backend must not act on them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub capability: String,
    pub selector: String,
    pub kit: String,
    pub request: String,
    pub socket: Option<String>,
    pub see: Option<Vec<String>>,
    pub allow: Option<Vec<String>>,
    pub remotes: Option<Vec<String>>,
    pub key: Option<Value>,
    pub via: Option<String>,
    pub notes: Option<Vec<String>>,
    pub env: Option<Vec<String>>,
    pub from: Option<Value>,
    pub holds: Option<String>,
    pub refresh: Option<String>,
    pub server: Option<String>,
    pub url: Option<String>,
    pub header: Option<String>,
    pub scheme: Option<String>,
    pub config: Option<String>,
}

// Info fields are parsed so that an unknown one is refused; a backend must not act on them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Child {
    pub slot: String,
    pub command: String,
    pub kit: String,
    pub kit_name: String,
    pub context: String,
    pub mode: String,
    pub file: String,
    pub hook: Option<String>,
    pub how: Option<String>,
    pub policy: Option<String>,
    pub argv: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub refused: Option<String>,
    pub text: Option<String>,
    pub grant: Option<bool>,
}

// Info fields are parsed so that an unknown one is refused; a backend must not act on them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: String,
    pub digest: Option<String>,
    #[serde(rename = "match")]
    pub matched: bool,
}

impl Policy {
    /// The effective items of KIND (a backend carries out only these).
    pub fn effective<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |i| i.effective && i.kind == kind)
    }

    pub fn proves(&self, guarantee: &str) -> bool {
        self.backend.proved.iter().any(|g| g == guarantee)
    }
}

/// Read policy.json/1 from TEXT: the version, the shape, then the rules a backend relies on.
pub fn parse(text: &str) -> Result<Policy, Refusal> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|e| Refusal::new("run", format!("policy.json is not JSON: {e}")))?;
    let context = raw
        .get("context")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .unwrap_or("run")
        .to_string();
    match raw.get("version") {
        Some(Value::Number(n)) if n.as_u64() == Some(1) => {}
        Some(v) => {
            return Err(Refusal::new(
                &context,
                format!("policy.json version {v}: this backend reads version 1 only"),
            ));
        }
        None => {
            return Err(Refusal::new(
                &context,
                "policy.json has no version: this backend reads version 1 only",
            ));
        }
    }
    let policy: Policy = decode(raw).map_err(|e| Refusal::new(&context, format!("policy.json/1: {e}")))?;
    check_rules(&policy).map_err(|m| Refusal::new(&context, format!("policy.json/1: {m}")))?;
    Ok(policy)
}

/// Decode V as T. An error names where it is (a JSON path) and what was expected there, never the value found:
/// a secret misplaced into a field would otherwise be printed (audit F7).
pub fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_path_to_error::deserialize(v).map_err(|e| {
        let path = e.path().to_string();
        let msg = e.inner().to_string();
        let said = if let Some((_, expected)) = msg
            .split_once(", expected ")
            .filter(|_| msg.starts_with("invalid "))
        {
            format!("expected {expected}")
        } else if msg.starts_with("unknown field") {
            msg.split(", expected").next().unwrap_or(&msg).to_string()
        } else if msg.starts_with("missing field") {
            msg
        } else {
            "not what this field holds".to_string()
        };
        if path == "." {
            said
        } else {
            format!("{path}: {said}")
        }
    })
}

/// Rule 1 (the backend's name) and rule 2's first half (every proved guarantee is one this contract knows).
pub fn check_backend(p: &Policy) -> Result<(), Refusal> {
    if p.backend.name != BACKEND {
        return Err(Refusal::new(
            &p.context,
            format!("policy.json is for the {} backend, not {BACKEND}", p.backend.name),
        ));
    }
    if let Some(g) = p
        .backend
        .proved
        .iter()
        .chain(&p.backend.requires)
        .find(|g| !GUARANTEES.contains(&g.as_str()))
    {
        return Err(Refusal::new(
            &p.context,
            format!("policy.json names a guarantee this backend does not know: {g}"),
        ));
    }
    if let Some(g) = p.backend.requires.iter().find(|g| !p.proves(g)) {
        return Err(Refusal::new(
            &p.context,
            format!("the context requires {g}, which this backend's probe did not prove"),
        ));
    }
    Ok(())
}

fn abs(p: &str) -> bool {
    p.starts_with('/')
}

fn env_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// `host:port`, `*` and `?` in the host: the schema's pattern for a network reach item.
pub fn host_port(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else {
        return false;
    };
    let mut h = host.chars();
    let first = h
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || "*?_".contains(c));
    first
        && h.all(|c| c.is_ascii_alphanumeric() || "*?._-".contains(c))
        && (1..=5).contains(&port.len())
        && port.chars().all(|c| c.is_ascii_digit())
}

#[allow(clippy::too_many_lines)] // one rule per schema conditional, in the schema's order
fn check_rules(p: &Policy) -> Result<(), String> {
    let run_ok =
        p.run == "explain" || (p.run.len() == 4 && p.run.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    if !run_ok {
        return Err(format!("run {:?} is not four hex digits", p.run));
    }
    if p.context.is_empty() {
        return Err("context is empty".into());
    }
    if !matches!(p.mode.as_str(), "in-place" | "clone") {
        return Err(format!("mode {:?} is not one this backend knows", p.mode));
    }
    if !matches!(p.access.as_str(), "write" | "read" | "none") {
        return Err(format!("access {:?} is not one this backend knows", p.access));
    }
    for w in &p.workspace {
        if !w.id.starts_with("repo:") || !abs(&w.path) || !abs(&w.common) {
            return Err(format!("workspace {}: id, path or common is malformed", w.path));
        }
        match (p.mode.as_str(), &w.clone) {
            ("in-place", Some(_)) => return Err(format!("workspace {}: a clone in mode in-place", w.path)),
            ("clone", None) => return Err(format!("workspace {}: no clone in mode clone", w.path)),
            ("clone", Some(c)) if !abs(c) => {
                return Err(format!("workspace {}: clone is not absolute", w.path));
            }
            _ => {}
        }
        if p.mode == "in-place" && !w.proposal.is_empty() {
            return Err(format!(
                "workspace {}: proposal patterns in mode in-place",
                w.path
            ));
        }
        if p.mode == "clone" && !w.sealed.is_empty() {
            return Err(format!("workspace {}: sealed patterns in a clone", w.path));
        }
        if p.access != "write" && !w.protected.is_empty() {
            return Err(format!(
                "workspace {}: protected patterns with access {}",
                w.path, p.access
            ));
        }
    }
    for i in &p.items {
        check_item(i)?;
    }
    if p.mounts.run != "/run/pot" || p.mounts.launcher != "/run/pot/launcher" {
        return Err("mounts.run must be /run/pot and mounts.launcher /run/pot/launcher".into());
    }
    for s in &p.mounts.state {
        if s.name.is_empty() || !s.at.starts_with("~/") || !abs(&s.dir) {
            return Err(format!("state mount {}: at or dir is malformed", s.name));
        }
    }
    for b in &p.bindings {
        check_binding(b)?;
    }
    for (k, c) in &p.children {
        let key_ok = k.rsplit_once(':').is_some_and(|(a, b)| {
            !a.is_empty()
                && !b.is_empty()
                && !a.contains(char::is_whitespace)
                && !b.contains(char::is_whitespace)
        });
        if !key_ok {
            return Err(format!("children: key {k:?} is not <kit id>:<command>"));
        }
        let admitted = c.how.is_some() || c.policy.is_some() || c.argv.is_some() || c.env.is_some();
        let refused = c.refused.is_some() || c.text.is_some() || c.grant.is_some();
        let full = c.how.is_some() && c.policy.is_some() && c.argv.is_some() && c.env.is_some();
        if admitted == refused || (admitted && !full) || (refused && c.refused.is_none()) {
            return Err(format!(
                "children: row {k} is neither admitted nor refused, or both"
            ));
        }
        if let Some(h) = &c.how {
            if !matches!(h.as_str(), "exec" | "sibling") {
                return Err(format!("children: row {k} has how {h:?}"));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // one rule per schema conditional, in the schema's order
fn check_item(i: &Item) -> Result<(), String> {
    let what = || format!("item {} {:?}", i.kind, i.value);
    if !matches!(i.class.as_str(), "see" | "limits" | "reach") {
        return Err(format!(
            "{}: class {:?} is not one this backend knows",
            what(),
            i.class
        ));
    }
    if i.sources.is_empty() {
        return Err(format!("{}: no sources", what()));
    }
    if matches!(i.class.as_str(), "see" | "limits") && !i.effective {
        return Err(format!("{}: a {} item that is not in effect", what(), i.class));
    }
    if !i.effective && i.why.is_none() {
        return Err(format!("{}: not in effect, and no why", what()));
    }
    if let Some((_, class)) = KINDS.iter().find(|(k, _)| *k == i.kind) {
        if !class.is_empty() && *class != i.class {
            return Err(format!(
                "{}: kind {} belongs to class {class}, not {}",
                what(),
                i.kind,
                i.class
            ));
        }
    }
    let need = |field: bool, name: &str| {
        if field {
            Ok(())
        } else {
            Err(format!("{}: no {name}", what()))
        }
    };
    match (i.kind.as_str(), i.class.as_str()) {
        ("network", "limits") if i.value != "none" => {
            return Err(format!("{}: a network limit other than none", what()));
        }
        ("network", "reach") if !host_port(&i.value) => return Err(format!("{}: not host:port", what())),
        ("network", "see") => return Err(format!("{}: network is limits or reach", what())),
        ("package", _) if !i.value.starts_with("/nix/store/") => {
            return Err(format!("{}: not a store path", what()));
        }
        ("env.set", _) if !i.value.split_once('=').is_some_and(|(n, _)| env_name(n)) => {
            return Err(format!("{}: not NAME=VALUE", what()));
        }
        ("access", _) if !matches!(i.value.as_str(), "write" | "read" | "none") => {
            return Err(format!("{}: not an access", what()));
        }
        ("hidden", _) => need(i.paths.is_some(), "paths")?,
        ("file", _) => {
            need(i.source.is_some() && i.mode.is_some(), "source or mode")?;
            if !(i.value.starts_with('/') || i.value.starts_with("~/")) {
                return Err(format!("{}: not / or ~/", what()));
            }
            if !matches!(i.mode.as_deref(), Some("fixed" | "seed" | "reassert")) {
                return Err(format!("{}: mode {:?}", what(), i.mode));
            }
        }
        ("dir", _) => {
            need(
                i.links.is_some() && i.mode.as_deref() == Some("fixed"),
                "links or mode fixed",
            )?;
            if !i.value.starts_with("~/") {
                return Err(format!("{}: not ~/", what()));
            }
        }
        ("read" | "write" | "socket", _) => need(i.path.is_some(), "path")?,
        ("state", _) => need(i.at.is_some() && i.shared.is_some(), "at or shared")?,
        ("capability", _) => {
            need(i.data.is_some(), "data")?;
            if !CAPABILITIES.contains(&i.value.as_str()) && !RESERVED.contains(&i.value.as_str()) {
                return Err(format!("{}: a capability this backend does not know", what()));
            }
            if i.effective {
                need(i.binding.is_some(), "binding")?;
            }
        }
        ("env", _) if !env_name(&i.value) => return Err(format!("{}: not a variable name", what())),
        ("view", _) => {
            need(
                i.home.is_some() && i.home_manager.is_some(),
                "home or homeManager",
            )?;
            if i.value != "inspect" {
                return Err(format!("{}: a view other than inspect", what()));
            }
        }
        // rule 4: a granted unsandboxed item runs on the host instead; a backend never runs one
        ("unsandboxed", _) if i.effective => {
            return Err(format!(
                "{}: an unsandboxed item in effect, which a backend never runs",
                what()
            ));
        }
        _ => {}
    }
    let paths = i
        .path
        .iter()
        .chain(i.source.iter())
        .chain(i.paths.iter().flatten());
    if let Some(bad) = paths.into_iter().find(|p| !abs(p)) {
        return Err(format!("{}: {bad:?} is not an absolute path", what()));
    }
    if i.at.as_deref().is_some_and(|a| !a.starts_with("~/")) {
        return Err(format!("{}: at is not ~/", what()));
    }
    Ok(())
}

fn check_binding(b: &Binding) -> Result<(), String> {
    let cap = b.capability.as_str();
    if RESERVED.contains(&cap) {
        return Err(format!(
            "a {cap} binding: the capability is reserved, and this backend does not carry it out"
        ));
    }
    if !CAPABILITIES.contains(&cap) {
        return Err(format!(
            "a {cap} binding: a capability this backend does not know"
        ));
    }
    if !b.request.starts_with("capability ") {
        return Err(format!("the {cap} binding's request is malformed"));
    }
    let need = |ok: bool, what: &str| {
        if ok {
            Ok(())
        } else {
            Err(format!("the {cap} binding has no {what}"))
        }
    };
    let socket = |want: &str| {
        if b.socket.as_deref() == Some(want) {
            Ok(())
        } else {
            Err(format!("the {cap} binding's socket is not {want}"))
        }
    };
    match cap {
        "status.relay" => {
            socket("/run/pot/relay.sock")?;
            need(b.see.is_some() && b.allow.is_some(), "see or allow")?;
        }
        "git.fetch" => {
            socket("/run/pot/git.sock")?;
            need(
                b.see.is_some() && b.remotes.is_some() && b.key.is_some(),
                "see, remotes or key",
            )?;
        }
        "nix" => {
            socket("/run/pot/nix.sock")?;
            need(
                b.see.is_some() && b.via.is_some() && b.notes.is_some(),
                "see, via or notes",
            )?;
        }
        _ => {
            // credential, mcp: references to the host's secrets, never a socket
            need(
                b.env.is_some() && b.from.is_some() && b.holds.is_some(),
                "env, from or holds",
            )?;
            if !matches!(b.via.as_deref(), Some("env" | "file")) {
                return Err(format!("the {cap} binding's via is not env or file"));
            }
            if b.socket.is_some() {
                return Err(format!("the {cap} binding has a socket"));
            }
            if cap == "mcp" {
                let all = b.server.is_some() && b.url.is_some() && b.header.is_some() && b.scheme.is_some();
                need(
                    all && b.config.as_deref() == Some("/run/pot/mcp.json"),
                    "server, url, header, scheme or config",
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_follows_the_schema() {
        assert!(host_port("api.anthropic.com:443"));
        assert!(host_port("*.example.test:8443"));
        assert!(!host_port("example.com"));
        assert!(!host_port("example.com:"));
        assert!(!host_port("example.com:123456"));
        assert!(!host_port(".x:1"));
    }

    #[test]
    fn version_is_checked_before_the_shape() {
        let e = parse(r#"{"version": 2, "context": "work", "new": 1}"#).unwrap_err();
        assert_eq!(
            e.to_string(),
            "pot (work): policy.json version 2: this backend reads version 1 only"
        );
        let e = parse(r#"{"context": "work"}"#).unwrap_err();
        assert_eq!(
            e.to_string(),
            "pot (work): policy.json has no version: this backend reads version 1 only"
        );
        let e = parse("{").unwrap_err();
        assert!(e.to_string().starts_with("pot (run): policy.json is not JSON"));
    }

    /// F7: a type error names where and what was expected, never the value (a misplaced secret would print).
    #[test]
    fn a_type_error_never_shows_the_value() {
        let text = include_str!("../../../docs/reference/container/prepare.in/read-only/policy.json");
        let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["items"][2]["effective"] = serde_json::json!("sk-live-123");
        let e = parse(&v.to_string()).unwrap_err().to_string();
        assert!(!e.contains("sk-live-123"), "{e}");
        assert_eq!(
            e,
            "pot (review): policy.json/1: items[2].effective: expected a boolean"
        );
        v["items"][2]["effective"] = serde_json::json!(true);
        v["bindings"] = serde_json::json!([{ "capability": "credential", "selector": "x", "kit": "k",
                                             "request": "capability credential", "value": "sk-live-123" }]);
        let e = parse(&v.to_string()).unwrap_err().to_string();
        assert!(!e.contains("sk-live-123"), "{e}");
        assert!(e.ends_with("bindings[0].value: unknown field `value`"), "{e}");
        v["bindings"] = serde_json::json!([]);
        v["run"] = serde_json::json!(["sk-live-123"]);
        let e = parse(&v.to_string()).unwrap_err().to_string();
        assert_eq!(e, "pot (review): policy.json/1: run: expected a string");
    }

    /// policy-1's schema names this backend in `backend.name` (a new value within version 1).
    #[test]
    fn the_schema_names_this_backend() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../contracts/policy-1.schema.json")).unwrap();
        let names = &schema["properties"]["backend"]["properties"]["name"]["enum"];
        assert!(names.as_array().unwrap().iter().any(|n| n == BACKEND), "{names}");
    }
}
