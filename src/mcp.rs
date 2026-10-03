//! MCP servers: one list in the canon (`mcp.toml`), written into each agent's
//! own MCP config, either for every project (the agent's global config) or for
//! one project (Claude's local scope, and the others' project files, which are
//! kept out of git).
//!
//! An entry carrying a server's name is the canon's: `fix` rewrites one that
//! differs from the canon and completes a project only some agents have it in,
//! and never adds one; `delete` takes them back. No token is ever in the canon:
//! a value names an environment variable, which each agent reads itself, or a
//! server says `token = true` and its token is kept in a private file outside
//! the canon (`Config::tokens`), which each agent reads on every connection.

use crate::config::{Agent, Config, tilde};
use crate::plan::{self, Cell, Change, State};
use crate::projects::{self, Projects};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A value a server is given: as written, or read from an environment
/// variable by the agent when it starts the server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Val {
    Lit(String),
    Env(String),
}

/// Where the token sent as `Authorization: Bearer` comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Bearer {
    /// An environment variable the agent reads when it starts.
    Env(String),
    /// The file canonize keeps the server's token in.
    Kept(PathBuf),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Spec {
    /// A program the agent starts and talks to over stdin and stdout.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, Val>,
    },
    /// A server reached over streamable HTTP.
    Http {
        url: String,
        bearer: Option<Bearer>,
        headers: BTreeMap<String, Val>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Server {
    pub name: String,
    pub spec: Spec,
}

impl Server {
    /// The environment variables the server needs, in the order they appear.
    pub fn vars(&self) -> Vec<&str> {
        fn env(m: &BTreeMap<String, Val>) -> impl Iterator<Item = &str> {
            m.values().filter_map(|v| match v {
                Val::Env(n) => Some(n.as_str()),
                Val::Lit(_) => None,
            })
        }
        match &self.spec {
            Spec::Stdio { env: e, .. } => env(e).collect(),
            Spec::Http {
                bearer, headers, ..
            } => bearer
                .iter()
                .filter_map(|b| match b {
                    Bearer::Env(n) => Some(n.as_str()),
                    Bearer::Kept(_) => None,
                })
                .chain(env(headers))
                .collect(),
        }
    }

    /// The file its token is kept in, when it has one there.
    pub fn kept(&self) -> Option<&Path> {
        match &self.spec {
            Spec::Http {
                bearer: Some(Bearer::Kept(p)),
                ..
            } => Some(p),
            _ => None,
        }
    }

    /// What it is, in a few words: its URL or its command.
    pub fn summary(&self) -> String {
        match &self.spec {
            Spec::Stdio { command, args, .. } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            Spec::Http { url, .. } => url.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer {
    url: Option<String>,
    bearer_env: Option<String>,
    #[serde(default)]
    token: bool,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// The canon's servers, in name order, their kept tokens in `tokens`. No file
/// means no servers.
pub fn load(file: &Path, tokens: &Path) -> Result<Vec<Server>> {
    if file.as_os_str().is_empty() {
        return Ok(Vec::new());
    }
    let text = match fs::read_to_string(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("could not read `{}`", tilde(file))),
        Ok(t) => t,
    };
    parse(&text, file, tokens)
}

/// The servers in `text`, which is `file`'s, for the errors to name it.
fn parse(text: &str, file: &Path, tokens: &Path) -> Result<Vec<Server>> {
    let raw: BTreeMap<String, RawServer> =
        toml::from_str(text).with_context(|| format!("`{}` is not valid", tilde(file)))?;
    raw.into_iter()
        .map(|(name, r)| {
            server(&name, r, tokens)
                .with_context(|| format!("server `{name}` in `{}`", tilde(file)))
        })
        .collect()
}

fn is_var(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// `${NAME}` as a whole is a variable; anything else holding `${` is refused,
/// because no agent but Claude can splice a variable into other text.
fn val(key: &str, s: &str) -> Result<Val> {
    if let Some(name) = s.strip_prefix("${").and_then(|x| x.strip_suffix('}'))
        && is_var(name)
    {
        return Ok(Val::Env(name.to_string()));
    }
    if s.contains("${") {
        bail!(
            "`{key}` mixes text and a variable: a value is plain text or one whole `${{NAME}}`, because that is all every agent can pass"
        );
    }
    Ok(Val::Lit(s.to_string()))
}

fn server(name: &str, r: RawServer, tokens: &Path) -> Result<Server> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("the name takes only letters, digits, `_` and `-`, which is all every agent accepts");
    }
    let spec = match (r.url, r.command) {
        (Some(_), Some(_)) => bail!("it has both `url` and `command`: keep the one it is"),
        (None, None) => bail!("it needs a `url` (a remote server) or a `command` (a local one)"),
        (Some(url), None) => {
            if !r.args.is_empty() || !r.env.is_empty() {
                bail!("`args` and `env` are for a server with a `command`");
            }
            if url.contains("${") {
                bail!("`url` is plain text: not every agent reads a variable in it");
            }
            if let Some(b) = &r.bearer_env
                && !is_var(b)
            {
                bail!("`bearer_env` is the name of an environment variable, such as `ESB_TOKEN`");
            }
            if (r.bearer_env.is_some() || r.token)
                && r.headers
                    .keys()
                    .any(|k| k.eq_ignore_ascii_case("authorization"))
            {
                bail!("it has a token and an `Authorization` header: keep one");
            }
            let bearer = match (r.bearer_env, r.token) {
                (Some(_), true) => bail!("it has `bearer_env` and `token = true`: keep one"),
                (Some(v), false) => Some(Bearer::Env(v)),
                (None, true) => Some(Bearer::Kept(tokens.join(name))),
                (None, false) => None,
            };
            let headers = r
                .headers
                .iter()
                .map(|(k, v)| Ok((k.clone(), val(&format!("headers.{k}"), v)?)))
                .collect::<Result<_>>()?;
            Spec::Http {
                url,
                bearer,
                headers,
            }
        }
        (None, Some(command)) => {
            if r.bearer_env.is_some() || r.token || !r.headers.is_empty() {
                bail!("`bearer_env`, `token` and `headers` are for a server with a `url`");
            }
            let mut env = BTreeMap::new();
            for (k, v) in &r.env {
                let v = val(&format!("env.{k}"), v)?;
                if let Val::Env(n) = &v
                    && n != k
                {
                    bail!(
                        "`env.{k}` reads `${{{n}}}`: a variable reaches a server under its own name in every agent, so write `{n} = \"${{{n}}}\"`"
                    );
                }
                env.insert(k.clone(), v);
            }
            Spec::Stdio {
                command,
                args: r.args,
                env,
            }
        }
    };
    Ok(Server {
        name: name.to_string(),
        spec,
    })
}

/// The keys of a server's table in mcp.toml that the form writes; any other
/// (`headers`, `env`) is kept when it is saved again.
const FORM_KEYS: [&str; 5] = ["url", "bearer_env", "token", "command", "args"];

/// Write server `name` into the canon's mcp.toml at `file`, keeping every
/// comment: a new one when `new`, refused if the name is taken, else over the
/// one there, keeping the keys the form does not show. Refuses a result
/// `load` would refuse.
pub fn define(file: &Path, name: &str, spec: &Spec, new: bool) -> Result<()> {
    use toml_edit::{Array, value};
    if file.as_os_str().is_empty() {
        bail!("MCP servers are turned off: `mcp` is \"\" in canonize.toml");
    }
    let mut doc = read_toml(file)?;
    if new && doc.contains_key(name) {
        bail!("your canon already has a server called `{name}`");
    }
    let mut t = match doc.get(name).and_then(toml_edit::Item::as_table) {
        Some(old) if !new => old.clone(),
        _ => toml_edit::Table::new(),
    };
    for k in FORM_KEYS {
        t.remove(k);
    }
    match spec {
        Spec::Http { url, bearer, .. } => {
            t["url"] = value(url.as_str());
            match bearer {
                Some(Bearer::Env(b)) => t["bearer_env"] = value(b.as_str()),
                Some(Bearer::Kept(_)) => t["token"] = value(true),
                None => {}
            }
        }
        Spec::Stdio { command, args, .. } => {
            t["command"] = value(command.as_str());
            if !args.is_empty() {
                t["args"] = value(args.iter().map(String::as_str).collect::<Array>());
            }
        }
    }
    doc.insert(name, toml_edit::Item::Table(t));
    let text = doc.to_string();
    parse(&text, file, Path::new(""))?;
    write(file, text)
}

/// Take server `name` out of the canon's mcp.toml at `file`, keeping every
/// other table and comment, and delete its kept token, which exists nowhere
/// else.
pub fn undefine(file: &Path, name: &str, token: Option<&Path>) -> Result<()> {
    let mut doc = read_toml(file)?;
    if doc.remove(name).is_some() {
        write(file, doc.to_string())?;
    }
    if let Some(t) = token
        && t.exists()
    {
        fs::remove_file(t).with_context(|| format!("could not delete `{}`", tilde(t)))?;
    }
    Ok(())
}

/// A token as RFC 6750 spells one, which every agent can carry in a header
/// and a shell can print without quoting.
pub fn is_token(t: &str) -> bool {
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~+/=".contains(c))
}

/// Keep `token` as the server's at `file`, readable by its owner alone.
pub fn save_token(file: &Path, token: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if !is_token(token) {
        bail!("a token is letters, digits and `-._~+/=`, with no spaces: paste it again");
    }
    if let Some(dir) = file.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("could not create `{}`", tilde(dir)))?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(file)
        .with_context(|| format!("could not write `{}`", tilde(file)))?;
    // A file made before under a looser umask keeps no wider access.
    f.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| format!("could not lock down `{}`", tilde(file)))?;
    f.write_all(token.as_bytes())
        .with_context(|| format!("could not write `{}`", tilde(file)))
}

/// What Claude runs for the headers of a server whose token is kept: its
/// `headersHelper`, run on every connection, so a new token takes effect on
/// the next one.
fn helper(kept: &Path) -> String {
    format!(
        "printf '{{\"Authorization\":\"Bearer %s\"}}' \"$(cat {})\"",
        sh_quote(kept)
    )
}

/// The header value pi runs for a kept token, its `!command` form.
fn pi_command(kept: &Path) -> String {
    format!("!echo Bearer $(cat {})", sh_quote(kept))
}

/// `path` as one shell word, whatever it holds: a home folder can have a
/// space in it.
fn sh_quote(path: &Path) -> String {
    sh_word(&path.display().to_string())
}

/// The path `sh_quote` wrote.
fn sh_unquote(word: &str) -> Option<PathBuf> {
    let inner = word.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(PathBuf::from(inner.replace("'\\''", "'")))
}

/// The variable codex reads a kept token from, since it cannot read a file:
/// `esb-mcp` gives `ESB_MCP_TOKEN`.
pub fn codex_var(kept: &Path) -> String {
    let name = kept
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{}_TOKEN", name.to_uppercase().replace('-', "_"))
}

/// `spec` as codex can have it: a kept token read from its variable instead.
fn for_codex(spec: &Spec) -> Spec {
    match spec {
        Spec::Http {
            url,
            bearer: Some(Bearer::Kept(p)),
            headers,
        } => Spec::Http {
            url: url.clone(),
            bearer: Some(Bearer::Env(codex_var(p))),
            headers: headers.clone(),
        },
        s => s.clone(),
    }
}

/// Where one agent keeps one server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Where {
    /// Claude's own state file, written only through `claude mcp`: for every
    /// project when `project` is `None`, else that project's local scope.
    Claude {
        state: PathBuf,
        /// The project's folder and the key Claude files it under.
        project: Option<(PathBuf, PathBuf)>,
    },
    /// `[mcp_servers.<name>]` in codex's config.toml.
    Codex(PathBuf),
    /// `mcpServers.<name>` in pi's mcp.json.
    Pi(PathBuf),
    /// `mcp.<name>` in an opencode.json.
    Opencode(PathBuf),
}

impl Where {
    /// The path two changes to the same place are told apart by.
    pub fn path(&self) -> &Path {
        match self {
            Where::Claude {
                project: Some((p, _)),
                ..
            } => p,
            Where::Claude { state, .. } => state,
            Where::Codex(f) | Where::Pi(f) | Where::Opencode(f) => f,
        }
    }

    /// Where it is, for a sentence.
    pub fn describe(&self) -> String {
        match self {
            Where::Claude { project: None, .. } => "Claude, for every project".to_string(),
            Where::Claude {
                project: Some((p, _)),
                ..
            } => format!("Claude in {}", tilde(p)),
            Where::Codex(f) | Where::Pi(f) | Where::Opencode(f) => tilde(f),
        }
    }
}

/// `spec` as Claude stores it, which pi reads too, without the `type`.
fn claude_value(spec: &Spec, typed: bool) -> Value {
    let vals = |m: &BTreeMap<String, Val>| -> Map<String, Value> {
        m.iter()
            .map(|(k, v)| {
                let s = match v {
                    Val::Lit(s) => s.clone(),
                    Val::Env(n) => format!("${{{n}}}"),
                };
                (k.clone(), Value::String(s))
            })
            .collect()
    };
    let mut o = Map::new();
    match spec {
        Spec::Stdio { command, args, env } => {
            o.insert("command".into(), json!(command));
            if !args.is_empty() {
                o.insert("args".into(), json!(args));
            }
            if !env.is_empty() {
                o.insert("env".into(), Value::Object(vals(env)));
            }
        }
        Spec::Http {
            url,
            bearer,
            headers,
        } => {
            if typed {
                o.insert("type".into(), json!("http"));
            }
            o.insert("url".into(), json!(url));
            let mut h = Map::new();
            match bearer {
                Some(Bearer::Env(b)) => {
                    h.insert("Authorization".into(), json!(format!("Bearer ${{{b}}}")));
                }
                Some(Bearer::Kept(p)) if typed => {
                    o.insert("headersHelper".into(), json!(helper(p)));
                }
                Some(Bearer::Kept(p)) => {
                    h.insert("Authorization".into(), json!(pi_command(p)));
                }
                None => {}
            }
            h.extend(vals(headers));
            if !h.is_empty() {
                o.insert("headers".into(), Value::Object(h));
            }
        }
    }
    Value::Object(o)
}

fn opencode_value(spec: &Spec) -> Value {
    let vals = |m: &BTreeMap<String, Val>| -> Map<String, Value> {
        m.iter()
            .map(|(k, v)| {
                let s = match v {
                    Val::Lit(s) => s.clone(),
                    Val::Env(n) => format!("{{env:{n}}}"),
                };
                (k.clone(), Value::String(s))
            })
            .collect()
    };
    let mut o = Map::new();
    match spec {
        Spec::Stdio { command, args, env } => {
            o.insert("type".into(), json!("local"));
            let mut cmd = vec![command.clone()];
            cmd.extend(args.iter().cloned());
            o.insert("command".into(), json!(cmd));
            if !env.is_empty() {
                o.insert("environment".into(), Value::Object(vals(env)));
            }
        }
        Spec::Http {
            url,
            bearer,
            headers,
        } => {
            o.insert("type".into(), json!("remote"));
            o.insert("url".into(), json!(url));
            let mut h = Map::new();
            match bearer {
                Some(Bearer::Env(b)) => {
                    h.insert("Authorization".into(), json!(format!("Bearer {{env:{b}}}")));
                }
                Some(Bearer::Kept(p)) => {
                    h.insert(
                        "Authorization".into(),
                        json!(format!("Bearer {{file:{}}}", p.display())),
                    );
                }
                None => {}
            }
            h.extend(vals(headers));
            if !h.is_empty() {
                o.insert("headers".into(), Value::Object(h));
            }
        }
    }
    Value::Object(o)
}

fn codex_table(spec: &Spec) -> toml_edit::Table {
    let spec = &for_codex(spec);
    use toml_edit::{Array, InlineTable, value};
    let mut t = toml_edit::Table::new();
    let inline = |pairs: Vec<(&String, &String)>| {
        let mut it = InlineTable::new();
        for (k, v) in pairs {
            it.insert(k, v.as_str().into());
        }
        value(it)
    };
    match spec {
        Spec::Stdio { command, args, env } => {
            t["command"] = value(command.as_str());
            if !args.is_empty() {
                t["args"] = value(args.iter().map(String::as_str).collect::<Array>());
            }
            let lit: Vec<_> = env
                .iter()
                .filter_map(|(k, v)| match v {
                    Val::Lit(s) => Some((k, s)),
                    Val::Env(_) => None,
                })
                .collect();
            if !lit.is_empty() {
                t["env"] = inline(lit);
            }
            let vars: Array = env
                .values()
                .filter_map(|v| match v {
                    Val::Env(n) => Some(n.as_str()),
                    Val::Lit(_) => None,
                })
                .collect();
            if !vars.is_empty() {
                t["env_vars"] = value(vars);
            }
        }
        Spec::Http {
            url,
            bearer,
            headers,
        } => {
            t["url"] = value(url.as_str());
            if let Some(Bearer::Env(b)) = bearer {
                t["bearer_token_env_var"] = value(b.as_str());
            }
            let split = |env: bool| -> Vec<(&String, &String)> {
                headers
                    .iter()
                    .filter_map(|(k, v)| match (v, env) {
                        (Val::Lit(s), false) | (Val::Env(s), true) => Some((k, s)),
                        _ => None,
                    })
                    .collect()
            };
            if !split(false).is_empty() {
                t["http_headers"] = inline(split(false));
            }
            if !split(true).is_empty() {
                t["env_http_headers"] = inline(split(true));
            }
        }
    }
    t
}

/// A string map whose values are read with `read`, or `None` when it is not one.
fn strings(v: Option<&Value>, read: impl Fn(&str) -> Val) -> Option<BTreeMap<String, Val>> {
    match v {
        None => Some(BTreeMap::new()),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| Some((k.clone(), read(v.as_str()?))))
            .collect(),
        Some(_) => None,
    }
}

fn wrapped<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    s.strip_prefix(open)
        .and_then(|x| x.strip_suffix(close))
        .filter(|x| is_var(x))
}

/// What a Claude or pi entry amounts to; `None` when it is nothing canonize writes.
fn from_claude(v: &Value) -> Option<Spec> {
    let read = |s: &str| match wrapped(s, "${", "}") {
        Some(n) => Val::Env(n.to_string()),
        None => Val::Lit(s.to_string()),
    };
    let str_list = |v: Option<&Value>| -> Option<Vec<String>> {
        match v {
            None => Some(Vec::new()),
            Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(String::from)).collect(),
            Some(_) => None,
        }
    };
    if let Some(url) = v.get("url") {
        if !matches!(
            v.get("type").and_then(Value::as_str),
            None | Some("http" | "streamable-http")
        ) {
            return None;
        }
        let mut headers = strings(v.get("headers"), read)?;
        let bearer = match headers.remove("Authorization") {
            None => None,
            Some(Val::Lit(s)) => {
                if let Some(n) = wrapped(&s, "Bearer ${", "}") {
                    Some(Bearer::Env(n.to_string()))
                } else if let Some(p) = s
                    .strip_prefix("!echo Bearer $(cat ")
                    .and_then(|x| x.strip_suffix(')'))
                    .and_then(sh_unquote)
                {
                    Some(Bearer::Kept(p))
                } else {
                    headers.insert("Authorization".into(), Val::Lit(s));
                    None
                }
            }
            Some(e) => {
                headers.insert("Authorization".into(), e);
                None
            }
        };
        let bearer = match (bearer, v.get("headersHelper")) {
            (None, Some(h)) => Some(Bearer::Kept(sh_unquote(
                h.as_str()?
                    .strip_prefix("printf '{\"Authorization\":\"Bearer %s\"}' \"$(cat ")?
                    .strip_suffix(")\"")?,
            )?)),
            (b, None) => b,
            (Some(_), Some(_)) => return None,
        };
        return Some(Spec::Http {
            url: url.as_str()?.to_string(),
            bearer,
            headers,
        });
    }
    if !matches!(v.get("type").and_then(Value::as_str), None | Some("stdio")) {
        return None;
    }
    Some(Spec::Stdio {
        command: v.get("command")?.as_str()?.to_string(),
        args: str_list(v.get("args"))?,
        env: strings(v.get("env"), read)?,
    })
}

fn from_opencode(v: &Value) -> Option<Spec> {
    let read = |s: &str| match wrapped(s, "{env:", "}") {
        Some(n) => Val::Env(n.to_string()),
        None => Val::Lit(s.to_string()),
    };
    match v.get("type")?.as_str()? {
        "remote" => {
            let mut headers = strings(v.get("headers"), read)?;
            let bearer = match headers.remove("Authorization") {
                Some(Val::Lit(s)) => {
                    if let Some(n) = wrapped(&s, "Bearer {env:", "}") {
                        Some(Bearer::Env(n.to_string()))
                    } else if let Some(p) = s
                        .strip_prefix("Bearer {file:")
                        .and_then(|x| x.strip_suffix('}'))
                    {
                        Some(Bearer::Kept(crate::config::expand(p)))
                    } else {
                        headers.insert("Authorization".into(), Val::Lit(s));
                        None
                    }
                }
                Some(e) => {
                    headers.insert("Authorization".into(), e);
                    None
                }
                None => None,
            };
            Some(Spec::Http {
                url: v.get("url")?.as_str()?.to_string(),
                bearer,
                headers,
            })
        }
        "local" => {
            let cmd: Vec<String> = v
                .get("command")?
                .as_array()?
                .iter()
                .map(|x| x.as_str().map(String::from))
                .collect::<Option<_>>()?;
            let (command, args) = cmd.split_first()?;
            Some(Spec::Stdio {
                command: command.clone(),
                args: args.to_vec(),
                env: strings(v.get("environment"), read)?,
            })
        }
        _ => None,
    }
}

fn from_codex(t: &toml::Value) -> Option<Spec> {
    let t = t.as_table()?;
    let map = |key: &str, env: bool| -> Option<BTreeMap<String, Val>> {
        match t.get(key) {
            None => Some(BTreeMap::new()),
            Some(v) => v
                .as_table()?
                .iter()
                .map(|(k, v)| {
                    let s = v.as_str()?.to_string();
                    Some((k.clone(), if env { Val::Env(s) } else { Val::Lit(s) }))
                })
                .collect(),
        }
    };
    if let Some(url) = t.get("url") {
        let mut headers = map("http_headers", false)?;
        headers.extend(map("env_http_headers", true)?);
        return Some(Spec::Http {
            url: url.as_str()?.to_string(),
            bearer: match t.get("bearer_token_env_var") {
                None => None,
                Some(b) => Some(Bearer::Env(b.as_str()?.to_string())),
            },
            headers,
        });
    }
    let mut env = map("env", false)?;
    if let Some(vars) = t.get("env_vars") {
        for v in vars.as_array()? {
            let n = v.as_str()?.to_string();
            env.insert(n.clone(), Val::Env(n));
        }
    }
    Some(Spec::Stdio {
        command: t.get("command")?.as_str()?.to_string(),
        args: match t.get("args") {
            None => Vec::new(),
            Some(a) => a
                .as_array()?
                .iter()
                .map(|x| x.as_str().map(String::from))
                .collect::<Option<_>>()?,
        },
        env,
    })
}

/// What is at `at` under `name` now: `Ok(None)` when nothing is, `Ok(Some(None))`
/// for an entry canonize cannot read as a server, and an error for a file
/// canonize will not touch.
fn current(at: &Where, name: &str, claude: Option<&Value>) -> Result<Option<Option<Spec>>, String> {
    let json = |file: &Path| -> Result<Option<Value>, String> {
        match fs::read_to_string(file) {
            Err(_) => Ok(None),
            Ok(t) => serde_json::from_str(&t).map(Some).map_err(|_| {
                format!(
                    "`{}` is not plain JSON, so canonize leaves it alone",
                    tilde(file)
                )
            }),
        }
    };
    Ok(match at {
        Where::Claude { project, .. } => {
            let Some(doc) = claude else {
                return Ok(None);
            };
            let servers = match project {
                None => doc.get("mcpServers"),
                Some((_, key)) => doc
                    .get("projects")
                    .and_then(|ps| ps.get(key.to_str()?))
                    .and_then(|e| e.get("mcpServers")),
            };
            servers.and_then(|s| s.get(name)).map(from_claude)
        }
        Where::Pi(f) => json(f)?
            .as_ref()
            .and_then(|d| d.get("mcpServers")?.get(name))
            .map(from_claude),
        Where::Opencode(f) => json(f)?
            .as_ref()
            .and_then(|d| d.get("mcp")?.get(name))
            .map(from_opencode),
        Where::Codex(f) => match fs::read_to_string(f) {
            Err(_) => None,
            Ok(t) => {
                let doc: toml::Table = toml::from_str(&t).map_err(|_| {
                    format!(
                        "`{}` is not valid TOML, so canonize leaves it alone",
                        tilde(f)
                    )
                })?;
                doc.get("mcp_servers")
                    .and_then(|s| s.get(name))
                    .map(from_codex)
            }
        },
    })
}

/// The keys canonize writes in an entry; any other key in it is the user's
/// and survives a rewrite.
fn managed(at: &Where) -> &'static [&'static str] {
    match at {
        Where::Codex(_) => &[
            "command",
            "args",
            "env",
            "env_vars",
            "url",
            "bearer_token_env_var",
            "http_headers",
            "env_http_headers",
        ],
        Where::Opencode(_) => &["type", "command", "environment", "url", "headers"],
        Where::Claude { .. } | Where::Pi(_) => &[
            "type",
            "command",
            "args",
            "env",
            "url",
            "headers",
            "headersHelper",
        ],
    }
}

/// Write `spec` as `name` at `at`, keeping whatever else is there.
pub fn set(at: &Where, name: &str, spec: &Spec) -> Result<()> {
    match at {
        Where::Claude { state, project } => {
            let doc = fs::read_to_string(state)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok());
            let dir = project.as_ref().map(|(p, _)| p.as_path());
            let old = doc.as_ref().and_then(|d| {
                match project {
                    None => d.get("mcpServers"),
                    Some((_, key)) => d.get("projects")?.get(key.to_str()?)?.get("mcpServers"),
                }?
                .get(name)
                .cloned()
            });
            let mut body = claude_value(spec, true);
            if let (Some(Value::Object(old)), Value::Object(new)) = (&old, &mut body) {
                for (k, v) in old {
                    if !managed(at).contains(&k.as_str()) {
                        new.insert(k.clone(), v.clone());
                    }
                }
            }
            if old.is_some() {
                claude(dir, &["mcp", "remove", "-s", scope(project), name])?;
            }
            let added = claude(
                dir,
                &[
                    "mcp",
                    "add-json",
                    "-s",
                    scope(project),
                    name,
                    &body.to_string(),
                ],
            );
            // `add-json` refuses a name that exists, so the old entry went
            // first; a failed add puts it back rather than leave nothing.
            if added.is_err()
                && let Some(old) = &old
            {
                let _ = claude(
                    dir,
                    &[
                        "mcp",
                        "add-json",
                        "-s",
                        scope(project),
                        name,
                        &old.to_string(),
                    ],
                );
            }
            added
        }
        Where::Codex(f) => {
            let mut doc = read_toml(f)?;
            let servers = doc
                .entry("mcp_servers")
                .or_insert_with(toml_edit::table)
                .as_table_mut()
                .with_context(|| format!("`mcp_servers` in `{}` is not a table", tilde(f)))?;
            servers.set_implicit(true);
            let mut t = codex_table(spec);
            if let Some(old) = servers.get(name).and_then(toml_edit::Item::as_table) {
                for (k, v) in old.iter() {
                    if !managed(at).contains(&k) {
                        t.insert(k, v.clone());
                    }
                }
            }
            servers.insert(name, toml_edit::Item::Table(t));
            write(f, doc.to_string())
        }
        Where::Pi(f) | Where::Opencode(f) => {
            let key = if matches!(at, Where::Pi(_)) {
                "mcpServers"
            } else {
                "mcp"
            };
            let value = match at {
                Where::Pi(_) => claude_value(spec, false),
                _ => opencode_value(spec),
            };
            let mut doc = read_json(f)?;
            let root = doc
                .as_object_mut()
                .with_context(|| format!("`{}` is not a JSON object", tilde(f)))?;
            let servers = root
                .entry(key)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .with_context(|| format!("`{key}` in `{}` is not an object", tilde(f)))?;
            let mut entry = match value {
                Value::Object(o) => o,
                _ => Map::new(),
            };
            if let Some(Value::Object(old)) = servers.get(name) {
                for (k, v) in old {
                    if !managed(at).contains(&k.as_str()) {
                        entry.insert(k.clone(), v.clone());
                    }
                }
            }
            servers.insert(name.to_string(), Value::Object(entry));
            write(f, serde_json::to_string_pretty(&doc)? + "\n")
        }
    }
}

/// Take `name` out of `at`. A file left with nothing in it is deleted, and so
/// is the project folder `in_project` it leaves empty. Anything else in the
/// file (a comment, a setting) keeps it, and no `.gitignore` line is touched:
/// either may be somebody else's.
pub fn remove(at: &Where, name: &str, in_project: bool) -> Result<()> {
    let emptied = match at {
        Where::Claude { project, .. } => {
            let dir = project.as_ref().map(|(p, _)| p.as_path());
            claude(dir, &["mcp", "remove", "-s", scope(project), name])?;
            return Ok(());
        }
        Where::Codex(f) => {
            let mut doc = read_toml(f)?;
            // toml_edit keeps the comments above a table as part of it, so
            // they are taken out of the table before it goes and kept.
            let mut kept = String::new();
            if let Some(servers) = doc
                .get_mut("mcp_servers")
                .and_then(toml_edit::Item::as_table_mut)
            {
                if let Some(t) = servers.get(name).and_then(toml_edit::Item::as_table) {
                    kept = t
                        .decor()
                        .prefix()
                        .and_then(|p| p.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                servers.remove(name);
                if servers.is_empty() {
                    doc.remove("mcp_servers");
                }
            }
            if kept.trim_start().starts_with('#') {
                let after = doc.trailing().as_str().unwrap_or_default().to_string();
                doc.set_trailing(format!("{kept}{after}"));
            }
            // A comment is the user's too, and `is_empty` does not see one.
            let text = doc.to_string();
            let empty = text.trim().is_empty();
            write(f, text)?;
            empty.then_some(f)
        }
        Where::Pi(f) | Where::Opencode(f) => {
            let key = if matches!(at, Where::Pi(_)) {
                "mcpServers"
            } else {
                "mcp"
            };
            let mut doc = read_json(f)?;
            let empty = match doc.as_object_mut() {
                Some(root) => {
                    if let Some(Value::Object(servers)) = root.get_mut(key) {
                        servers.shift_remove(name);
                        if servers.is_empty() {
                            root.shift_remove(key);
                        }
                    }
                    root.is_empty()
                }
                None => false,
            };
            write(f, serde_json::to_string_pretty(&doc)? + "\n")?;
            empty.then_some(f)
        }
    };
    if let Some(f) = emptied {
        fs::remove_file(f).with_context(|| format!("could not delete `{}`", tilde(f)))?;
        // A project's `.codex/` or `.pi/` left empty is debris; an agent's own
        // folder is its home and stays.
        if in_project
            && let Some(dir) = f.parent()
            && fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none())
        {
            fs::remove_dir(dir).with_context(|| format!("could not delete `{}`", tilde(dir)))?;
        }
    }
    Ok(())
}

fn scope<T>(project: &Option<T>) -> &'static str {
    if project.is_some() { "local" } else { "user" }
}

/// Run `claude` with `args`, in `project` for its local scope.
fn claude(project: Option<&Path>, args: &[&str]) -> Result<()> {
    let mut cmd = std::process::Command::new("claude");
    cmd.args(args);
    if let Some(p) = project {
        cmd.current_dir(p);
    }
    let out = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => anyhow::anyhow!(
            "`claude` is not on your PATH, so canonize cannot change Claude's MCP servers"
        ),
        _ => anyhow::anyhow!(
            "could not run `claude`: {}",
            e.to_string()
                .split(" (os error")
                .next()
                .unwrap_or("it failed")
        ),
    })?;
    if out.status.success() {
        return Ok(());
    }
    let said =
        String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    let said: Vec<&str> = said
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .collect();
    bail!(
        "`claude {}` failed: {}",
        args[..2].join(" "),
        said.join("\n")
    )
}

fn read_toml(f: &Path) -> Result<toml_edit::DocumentMut> {
    match plan::read_text(f)? {
        None => Ok(toml_edit::DocumentMut::new()),
        Some(t) => t
            .parse()
            .with_context(|| format!("`{}` is not valid TOML", tilde(f))),
    }
}

fn read_json(f: &Path) -> Result<Value> {
    match plan::read_text(f)? {
        None => Ok(Value::Object(Map::new())),
        Some(t) => serde_json::from_str(&t).with_context(|| {
            format!(
                "`{}` is not plain JSON, so canonize leaves it alone",
                tilde(f)
            )
        }),
    }
}

fn write(f: &Path, body: String) -> Result<()> {
    if let Some(parent) = f.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create `{}`", tilde(parent)))?;
    }
    fs::write(f, body).with_context(|| format!("could not write `{}`", tilde(f)))
}

/// The shell lines that do the same as setting `name` at `at`.
pub fn command(at: &Where, name: &str, spec: Option<&Spec>) -> String {
    match (at, spec) {
        (Where::Claude { project, .. }, spec) => {
            let cd = project
                .as_ref()
                .map_or(String::new(), |(p, _)| format!("cd {} && ", tilde(p)));
            let s = scope(project);
            let remove = format!("{cd}claude mcp remove -s {s} {name}");
            match spec {
                // `add-json` refuses a name that is there, so what is there goes first.
                Some(spec) => format!(
                    "{remove} 2>/dev/null\n{cd}claude mcp add-json -s {s} {name} {}",
                    sh_word(&claude_value(spec, true).to_string())
                ),
                None => remove,
            }
        }
        // Codex's TOML has no tool to set one table, so the table is shown for
        // what it is: what canonize writes there, over any table of that name.
        (Where::Codex(f), Some(spec)) => {
            let mut doc = toml_edit::DocumentMut::new();
            let mut servers = toml_edit::Table::new();
            servers.set_implicit(true);
            servers.insert(name, toml_edit::Item::Table(codex_table(spec)));
            doc.insert("mcp_servers", toml_edit::Item::Table(servers));
            let table: String = doc
                .to_string()
                .lines()
                .map(|l| format!("\n#   {l}"))
                .collect();
            format!("# set in {}:{table}", tilde(f))
        }
        (Where::Codex(f), None) => format!("# delete [mcp_servers.{name}] from {}", tilde(f)),
        (Where::Pi(f) | Where::Opencode(f), spec) => {
            let key = if matches!(at, Where::Pi(_)) {
                "mcpServers"
            } else {
                "mcp"
            };
            let jq = match spec {
                Some(spec) => {
                    let v = match at {
                        Where::Pi(_) => claude_value(spec, false),
                        _ => opencode_value(spec),
                    };
                    format!(".{key}[\"{name}\"] = {v}")
                }
                None => format!("del(.{key}[\"{name}\"])"),
            };
            let input = if f.exists() {
                tilde(f)
            } else {
                "<(echo '{}')".to_string()
            };
            format!(
                "jq {} {input} > {f}.new && mv {f}.new {f}",
                sh_word(&jq),
                f = tilde(f)
            )
        }
    }
}

/// `text` as one single-quoted shell word.
fn sh_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The canon's servers against every agent and every project.
pub struct Mcps {
    pub servers: Vec<Server>,
    /// Why mcp.toml could not be read, when it could not.
    pub error: Option<String>,
    /// `cells[server][agent]`, agents in config order: the agent's global config.
    pub cells: Vec<Vec<Cell>>,
    /// Per project, in the order of `Projects::list`, its cell per server.
    pub projects: Vec<Vec<projects::Cell>>,
}

/// Where `agent` keeps a server: its global config, or with `project` (the
/// folder and Claude's key for it) that project's own. A project file comes
/// with its path relative to the project, for `.gitignore`.
fn place(
    cfg: &Config,
    agent: &Agent,
    project: Option<(&Path, &Path)>,
) -> Option<(Where, Option<String>)> {
    let file = |home_rel: &str, project_rel: &str| match project {
        None => (agent.home.join(home_rel), None),
        Some((p, _)) => (p.join(project_rel), Some(project_rel.to_string())),
    };
    Some(match agent.name.as_str() {
        "claude" => (
            Where::Claude {
                state: cfg.claude_state.clone(),
                project: project.map(|(p, k)| (p.to_path_buf(), k.to_path_buf())),
            },
            None,
        ),
        "codex" => {
            let (f, rel) = file("config.toml", ".codex/config.toml");
            (Where::Codex(f), rel)
        }
        "pi" => {
            let (f, rel) = file("mcp.json", ".pi/mcp.json");
            (Where::Pi(f), rel)
        }
        "opencode" => {
            let (f, rel) = file("opencode.json", "opencode.json");
            (Where::Opencode(f), rel)
        }
        _ => return None,
    })
}

/// One agent's entry for `server` at `at`, as a cell; `in_project` when the
/// file is a project's.
fn entry(at: &Where, server: &Server, in_project: bool, claude: Option<&Value>) -> Cell {
    let path = at.path().to_path_buf();
    let undo = || Change::McpRemove {
        at: at.clone(),
        name: server.name.clone(),
        in_project,
    };
    let set = || Change::McpSet {
        at: at.clone(),
        name: server.name.clone(),
        spec: server.spec.clone(),
    };
    // Codex reads a kept token from a variable, so that is what it should have.
    let want = match at {
        Where::Codex(_) => for_codex(&server.spec),
        _ => server.spec.clone(),
    };
    match current(at, &server.name, claude) {
        Err(why) => plan::absent(path, State::Foreign(why)),
        Ok(None) => Cell {
            state: State::Missing,
            change: Some(set()),
            undo: None,
            at: path,
        },
        Ok(Some(Some(spec))) if spec == want => Cell {
            state: State::Linked,
            change: None,
            undo: Some(undo()),
            at: path,
        },
        Ok(Some(_)) => Cell {
            state: State::Broken("differs from your canon".into()),
            change: Some(set()),
            undo: Some(undo()),
            at: path,
        },
    }
}

/// Where each agent keeps a project's servers, worked out once per project:
/// `Err` says why an agent's project file is refused.
type Slot = (String, Result<(Where, Option<Change>, bool), String>);

fn slots(cfg: &Config, root: &Path) -> Vec<Slot> {
    let key = projects::claude_key(root);
    cfg.agents
        .iter()
        .filter(|a| a.active())
        // Claude keeps one list per git repo, so a project below its repo's
        // root shares it with every other project there and gets no say in it.
        .filter(|a| a.name != "claude" || key == root)
        .filter_map(|agent| {
            let (at, rel) = place(cfg, agent, Some((root, &key)))?;
            let Some(rel) = rel else {
                return Some((agent.name.clone(), Ok((at, None, false))));
            };
            let file = root.join(&rel);
            let slot = if plan::is_link(&file) {
                Err(format!("{rel} is a link, so canonize leaves it alone"))
            } else if file.exists() && projects::tracked(root, &rel) {
                Err(format!(
                    "{rel} is tracked by git, so canonize leaves it alone: `git rm --cached {rel}`, then try again"
                ))
            } else {
                let wire = projects::ignored(root, &rel, false);
                Ok((at, wire.change, true))
            };
            Some((agent.name.clone(), slot))
        })
        .collect()
}

/// One project's cell for `server`, over every agent that can take it: added
/// when every one has it, `-` when none does, broken in between or when one
/// differs from the canon.
fn project_cell(
    root: &Path,
    slots: &[Slot],
    server: &Server,
    claude: Option<&Value>,
) -> projects::Cell {
    let mut subs = Vec::new();
    let mut refused = Vec::new();
    for (agent, slot) in slots {
        match slot {
            Err(why) => refused.push(format!("{agent}: {why}")),
            Ok((at, wire, in_project)) => {
                let cell = entry(at, server, *in_project, claude);
                match &cell.state {
                    State::Foreign(why) => refused.push(format!("{agent}: {why}")),
                    _ => subs.push((agent.as_str(), cell, wire)),
                }
            }
        }
    }
    if subs.is_empty() {
        return projects::Cell {
            state: if refused.is_empty() {
                State::Na
            } else {
                State::Foreign(refused.join("; "))
            },
            change: None,
            undo: None,
        };
    }
    let changes: Vec<Change> = subs
        .iter()
        .filter(|(_, c, _)| c.state != State::Linked)
        // The entry first, then its `.gitignore` line: a batch is known by its
        // first change, and the line is the same for every server here.
        .flat_map(|(_, c, w)| c.change.clone().into_iter().chain(w.iter().cloned()))
        .collect();
    let undos: Vec<Change> = subs.iter().filter_map(|(_, c, _)| c.undo.clone()).collect();
    let batch = |what: String, changes: Vec<Change>| -> Option<Change> {
        let mut changes = plan::dedup(changes.into_iter());
        match changes.len() {
            0 => None,
            1 => changes.pop(),
            _ => Some(Change::Batch { what, changes }),
        }
    };
    let state = if subs.iter().all(|(_, c, _)| c.state == State::Linked) {
        State::Linked
    } else if subs.iter().all(|(_, c, _)| c.state == State::Missing) {
        State::Missing
    } else {
        let names = |f: &dyn Fn(&State) -> bool| -> Vec<&str> {
            subs.iter()
                .filter(|(_, c, _)| f(&c.state))
                .map(|(n, _, _)| *n)
                .collect()
        };
        let missing = names(&|s| *s == State::Missing);
        let differs = names(&|s| matches!(s, State::Broken(_)));
        let mut why = Vec::new();
        if !missing.is_empty() {
            why.push(format!("missing for {}", missing.join(", ")));
        }
        if !differs.is_empty() {
            why.push(format!(
                "differs from your canon for {}",
                differs.join(", ")
            ));
        }
        State::Broken(why.join(", "))
    };
    projects::Cell {
        change: batch(
            format!(
                "add MCP server `{}` for every agent in {}",
                server.name,
                tilde(root)
            ),
            changes,
        ),
        undo: batch(
            format!(
                "delete MCP server `{}` from every agent in {}",
                server.name,
                tilde(root)
            ),
            undos,
        ),
        state,
    }
}

impl Mcps {
    pub fn build(cfg: &Config, projects: &Projects) -> Mcps {
        let (servers, error) = match load(&cfg.source.mcp, &cfg.tokens) {
            Ok(s) => (s, None),
            Err(e) => (Vec::new(), Some(format!("{e:#}"))),
        };
        // Read once: Claude's state file can run to megabytes.
        let claude = (!servers.is_empty())
            .then(|| fs::read_to_string(&cfg.claude_state).ok())
            .flatten()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let claude = claude.as_ref();
        let cells = servers
            .iter()
            .map(|s| {
                cfg.agents
                    .iter()
                    .map(|a| {
                        if !a.active() {
                            return plan::absent(a.home.clone(), State::Absent);
                        }
                        match place(cfg, a, None) {
                            Some((at, _)) => entry(&at, s, false, claude),
                            None => plan::absent(a.home.clone(), State::Na),
                        }
                    })
                    .collect()
            })
            .collect();
        let projects = projects
            .list
            .iter()
            .map(|p| {
                if servers.is_empty() {
                    return Vec::new();
                }
                let slots = slots(cfg, &p.root);
                servers
                    .iter()
                    .map(|s| project_cell(&p.root, &slots, s, claude))
                    .collect()
            })
            .collect();
        Mcps {
            servers,
            error,
            cells,
            projects,
        }
    }

    fn of(&self, only: Option<usize>) -> impl Iterator<Item = &Cell> {
        self.cells.iter().flat_map(move |row| {
            row.iter()
                .enumerate()
                .filter(move |(a, _)| only.is_none_or(|o| o == *a))
                .map(|(_, c)| c)
        })
    }

    /// The projects' cells, which count only when no single agent is asked about.
    fn in_projects(&self, only: Option<usize>) -> impl Iterator<Item = &projects::Cell> {
        self.projects
            .iter()
            .flatten()
            .filter(move |_| only.is_none())
    }

    /// What `fix` does: rewrite what differs and complete a project only some
    /// agents have a server in, and nothing else.
    pub fn fixes(&self, only: Option<usize>) -> Vec<Change> {
        let broken = |s: &State| matches!(s, State::Broken(_));
        plan::dedup(
            self.of(only)
                .filter(|c| broken(&c.state))
                .filter_map(|c| c.change.clone())
                .chain(
                    self.in_projects(only)
                        .filter(|c| broken(&c.state))
                        .filter_map(|c| c.change.clone()),
                ),
        )
    }

    pub fn drifted(&self, only: Option<usize>) -> bool {
        let broken = |s: &State| matches!(s, State::Broken(_));
        self.of(only).any(|c| broken(&c.state)) || self.in_projects(only).any(|c| broken(&c.state))
    }

    /// Every entry of a canon server, for `canon delete`.
    pub fn undos(&self, only: Option<usize>) -> Vec<Change> {
        plan::dedup(
            self.of(only)
                .filter_map(|c| c.undo.clone())
                .chain(self.in_projects(only).filter_map(|c| c.undo.clone())),
        )
    }

    /// What a person has to know about the kept tokens of the servers some
    /// agent has: one with no token yet, and the variable codex reads instead.
    pub fn token_notes(&self, cfg: &Config) -> Vec<String> {
        let mut out = Vec::new();
        for (i, s) in self.servers.iter().enumerate() {
            let Some(kept) = s.kept() else { continue };
            let used = self.cells[i].iter().any(|c| c.state == State::Linked)
                || self.projects.iter().any(|p| p[i].state == State::Linked);
            if !used {
                continue;
            }
            if !kept.is_file() {
                out.push(format!(
                    "{}: no token yet, so it is sent none: e on it in the MCPs tab sets one",
                    s.name
                ));
            }
            let codex = cfg.agents.iter().position(|a| a.name == "codex");
            if codex.is_some_and(|a| self.cells[i][a].state == State::Linked) {
                let var = codex_var(kept);
                let unset = std::env::var_os(&var).is_none_or(|v| v.is_empty());
                out.push(format!(
                    "{}: codex reads its token from `{var}`, since it cannot read a file{}",
                    s.name,
                    if unset {
                        ", and that is not set here"
                    } else {
                        ""
                    }
                ));
            }
        }
        out
    }

    /// The variables a server somebody uses needs and this environment lacks,
    /// as `(server, variable)`.
    pub fn unset(&self) -> Vec<(&str, &str)> {
        self.servers
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                self.cells[*i].iter().any(|c| c.state == State::Linked)
                    || self.projects.iter().any(|p| p[*i].state == State::Linked)
            })
            .flat_map(|(_, s)| {
                s.vars()
                    .into_iter()
                    .filter(|v| std::env::var_os(v).is_none_or(|x| x.is_empty()))
                    .map(move |v| (s.name.as_str(), v))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RulesMode, SkillsMode};
    use crate::tmp::{self, Temp};

    /// codex, pi and opencode installed (Claude's side runs the real `claude`,
    /// which no test does), a canon with `servers` as its mcp.toml, and one
    /// git project `dev/app`.
    fn world(servers: &str) -> (Temp, Config) {
        let t = Temp::new();
        t.write("canon/mcp.toml", servers);
        let agent = |name: &str| {
            let home = t.dir(&format!("home/{name}"));
            tmp::agent(
                name,
                &home,
                &home.join("AGENTS.md"),
                RulesMode::Link,
                &home.join("skills"),
                SkillsMode::Folder,
            )
        };
        let agents = vec![agent("codex"), agent("pi"), agent("opencode")];
        t.write("dev/app/AGENTS.md", "# app\n");
        t.git("dev/app");
        let cfg = tmp::config(tmp::source(&t.at("canon")), agents, vec![t.at("dev")]);
        (t, cfg)
    }

    const ESB: &str = "[esb]\nurl = \"https://esb.example.com/mcp\"\nbearer_env = \"ESB_TOKEN\"\n";

    fn build(cfg: &Config) -> Mcps {
        Mcps::build(cfg, &Projects::build(cfg))
    }

    fn run(changes: impl IntoIterator<Item = Change>) {
        for c in changes {
            c.run().expect("a change the plan offered should run");
        }
    }

    #[test]
    fn a_server_takes_only_what_every_agent_can_pass() {
        let t = Temp::new();
        for (toml, why) in [
            (
                "[a]\nurl = \"https://example.com/${X}\"\n",
                "a variable in a url",
            ),
            (
                "[a]\nurl = \"https://example.com\"\nheaders = { X = \"key ${K}\" }\n",
                "text around a variable",
            ),
            (
                "[a]\ncommand = \"x\"\nenv = { API_KEY = \"${MY_KEY}\" }\n",
                "a variable under another name",
            ),
            (
                "[a]\ncommand = \"x\"\nurl = \"https://example.com\"\n",
                "both a url and a command",
            ),
            ("[a]\nargs = [\"x\"]\n", "neither a url nor a command"),
            ("[\"a b\"]\ncommand = \"x\"\n", "a space in the name"),
        ] {
            let file = t.write("mcp.toml", toml);
            assert!(
                load(&file, Path::new("")).is_err(),
                "{why} should be refused"
            );
        }
        let file = t.write(
            "mcp.toml",
            "[a]\ncommand = \"x\"\nenv = { API_KEY = \"${API_KEY}\", DEBUG = \"1\" }\n",
        );
        assert!(
            load(&file, Path::new("")).is_ok(),
            "a variable under its own name is fine"
        );
    }

    #[test]
    fn a_new_server_goes_into_the_canon_keeping_what_is_there() {
        let t = Temp::new();
        let file = t.write(
            "mcp.toml",
            "# my servers\n\n[esb]\n# work\nurl = \"https://esb.example.com/mcp\"\n",
        );
        let spec = Spec::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "fs".into()],
            env: BTreeMap::new(),
        };

        define(&file, "files", &spec, true).expect("a new name should go in");
        assert!(
            define(&file, "esb", &spec, true).is_err(),
            "a name the canon has is refused"
        );

        let text = fs::read_to_string(&file).expect("mcp.toml should be there");
        assert!(
            text.contains("# my servers") && text.contains("# work"),
            "{text}"
        );
        let servers = load(&file, Path::new("")).expect("the result should load");
        assert_eq!(
            servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["esb", "files"]
        );
        assert_eq!(servers[1].spec, spec, "it reads back as written");
    }

    #[test]
    fn a_kept_token_reaches_every_agent_without_landing_in_a_config() {
        let (t, cfg) = world("[esb]\nurl = \"https://esb.example.com/mcp\"\ntoken = true\n");
        save_token(&cfg.tokens.join("esb"), "s3cr3t-token").expect("the token should be kept");
        run(build(&cfg)
            .cells
            .iter()
            .flatten()
            .filter_map(|c| c.change.clone()));

        let m = build(&cfg);
        for (a, c) in cfg.agents.iter().zip(&m.cells[0]) {
            assert!(
                c.state == State::Linked,
                "{} should read back esb as added, not {}",
                a.name,
                c.state.word()
            );
        }
        for file in [
            "home/pi/mcp.json",
            "home/opencode/opencode.json",
            "home/codex/config.toml",
        ] {
            let text = fs::read_to_string(t.at(file)).expect("the agent's config should be there");
            assert!(
                !text.contains("s3cr3t-token"),
                "{file} holds the token itself: {text}"
            );
        }
        let codex = fs::read_to_string(t.at("home/codex/config.toml")).expect("codex's config");
        assert!(
            codex.contains("bearer_token_env_var = \"ESB_TOKEN\""),
            "codex, which reads no file, is pointed at a variable: {codex}"
        );
    }

    #[test]
    fn a_kept_token_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let t = Temp::new();
        let file = t.at("tokens/esb");
        save_token(&file, "first").expect("the token should be kept");
        save_token(&file, "second").expect("a new token replaces it");

        assert_eq!(fs::read_to_string(&file).expect("kept"), "second");
        let mode = |p: &Path| fs::metadata(p).expect("there").permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600, "the token file is the owner's alone");
        assert_eq!(mode(&t.at("tokens")), 0o700, "and so is its folder");
        assert!(
            save_token(&file, "has a space").is_err(),
            "what no header can carry is refused"
        );
    }

    #[test]
    fn deleting_a_server_from_the_canon_takes_its_token_and_nothing_else() {
        let t = Temp::new();
        let file = t.write(
            "mcp.toml",
            "# my servers\n\n[docs]\nurl = \"https://docs.example.com/mcp\"\n\n[esb]\nurl = \"https://esb.example.com/mcp\"\ntoken = true\n",
        );
        let token = t.at("tokens/esb");
        save_token(&token, "s3cr3t").expect("the token should be kept");

        undefine(&file, "esb", Some(&token)).expect("the server should go");

        let text = fs::read_to_string(&file).expect("mcp.toml should be there");
        assert!(!text.contains("[esb]"), "{text}");
        assert!(
            text.contains("# my servers") && text.contains("[docs]"),
            "{text}"
        );
        assert!(!token.exists(), "its kept token goes with it");
    }

    #[test]
    fn a_server_written_for_each_agent_reads_back_as_added() {
        let local = "[files]\ncommand = \"npx\"\nargs = [\"-y\", \"fs\"]\nenv = { DEBUG = \"1\", API_KEY = \"${API_KEY}\" }\n";
        let (_t, cfg) = world(&format!("{ESB}{local}"));
        let m = build(&cfg);
        run(m.cells.iter().flatten().filter_map(|c| c.change.clone()));

        let m = build(&cfg);
        for (s, row) in m.servers.iter().zip(&m.cells) {
            for (a, c) in cfg.agents.iter().zip(row) {
                assert!(
                    c.state == State::Linked,
                    "{} should read back {} as added, not {}",
                    a.name,
                    s.name,
                    c.state.word()
                );
            }
        }
        assert!(!m.drifted(None));
    }

    #[test]
    fn a_rewrite_keeps_the_keys_the_user_added() {
        let (t, cfg) = world(ESB);
        run(build(&cfg)
            .cells
            .iter()
            .flatten()
            .filter_map(|c| c.change.clone()));
        let pi = t.at("home/pi/mcp.json");
        let text = fs::read_to_string(&pi).expect("pi's mcp.json should be written");
        fs::write(
            &pi,
            text.replace("\"url\"", "\"timeout\": 30,\n      \"url\""),
        )
        .expect("edit");

        t.write(
            "canon/mcp.toml",
            &ESB.replace("esb.example.com", "esb2.example.com"),
        );
        let m = build(&cfg);
        assert!(m.drifted(None), "a server changed in the canon is drift");
        run(m.fixes(None));

        let text = fs::read_to_string(&pi).expect("pi's mcp.json should be there");
        assert!(text.contains("esb2.example.com"), "fix rewrote it: {text}");
        assert!(
            text.contains("\"timeout\": 30"),
            "the user's key stayed: {text}"
        );
        assert!(!build(&cfg).drifted(None));
    }

    #[test]
    fn a_project_file_git_tracks_is_never_written() {
        let (t, cfg) = world(ESB);
        let root = t.at("dev/app");
        fs::remove_dir_all(root.join(".git")).expect("drop the fake repo");
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.email=a@example.com", "-c", "user.name=a"])
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success());
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q"]);
        t.write("dev/app/opencode.json", "{\"theme\": \"x\"}\n");
        git(&["add", "opencode.json"]);
        git(&["commit", "-qm", "init"]);

        let m = build(&cfg);
        run(m.projects[0][0].change.clone());

        assert_eq!(
            fs::read_to_string(root.join("opencode.json")).expect("still there"),
            "{\"theme\": \"x\"}\n",
            "a tracked opencode.json is the team's"
        );
        assert!(root.join(".pi/mcp.json").is_file(), "pi still gets it");
        assert!(
            root.join(".codex/config.toml").is_file(),
            "codex still gets it"
        );
    }

    #[test]
    fn taking_a_server_out_of_a_project_leaves_no_file_of_it() {
        let (t, cfg) = world(ESB);
        run(build(&cfg).projects[0][0].change.clone());
        assert!(t.at("dev/app/.pi/mcp.json").is_file());

        run(build(&cfg).undos(None));

        for left in [".codex", ".pi", "opencode.json"] {
            assert!(
                !t.at("dev/app").join(left).exists(),
                "{left} should be gone"
            );
        }
    }

    #[test]
    fn taking_a_server_out_keeps_whatever_else_its_file_holds() {
        let (t, cfg) = world(ESB);
        t.write(
            "home/codex/config.toml",
            "# my codex settings, keep these\n\n[mcp_servers.esb]\nurl = \"https://esb.example.com/mcp\"\nbearer_token_env_var = \"ESB_TOKEN\"\n",
        );
        let ignore = "target/\n.codex/config.toml\n";
        t.write("dev/app/.gitignore", ignore);
        run(build(&cfg).projects[0][0].change.clone());

        run(build(&cfg).undos(None));

        let codex = fs::read_to_string(t.at("home/codex/config.toml"))
            .expect("a file with a comment left in it is the user's and stays");
        assert!(codex.contains("# my codex settings"), "{codex}");
        assert!(!codex.contains("esb"), "only the server went: {codex}");
        assert_eq!(
            fs::read_to_string(t.at("dev/app/.gitignore")).expect(".gitignore stays"),
            format!("{ignore}.pi/mcp.json\nopencode.json\n"),
            "no .gitignore line is taken out, the user's least of all"
        );
    }
}
