//! Where the source lives, what is in it, and which agents it reaches.
//!
//! The source is a folder the user owns (usually inside their dotfiles) holding
//! `canonize.toml`, the rules file, house files and skills. Every agent has
//! built-in defaults for where it reads rules and skills; `canonize.toml`
//! overrides any of them and can add agents the defaults do not know.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "canonize.toml";

/// How the rules file reaches an agent.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum RulesMode {
    /// An `@path` line in the agent's own instructions file, which the agent
    /// resolves itself. The file keeps everything else the user wrote in it.
    Import,
    /// The agent's instructions file is a symlink to the rules file.
    Link,
    Off,
}

/// How skills reach an agent.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum SkillsMode {
    /// One symlink per skill inside a folder the agent also writes to.
    PerSkill,
    /// The agent's skills folder itself is a symlink to the source's.
    Folder,
    Off,
}

pub struct Source {
    pub root: PathBuf,
    pub rules: PathBuf,
    pub schema: PathBuf,
    /// A pattern relative to `root` with at most one `*` in its file name.
    pub house: String,
    pub skills: PathBuf,
}

pub struct Agent {
    pub name: String,
    pub enabled: bool,
    /// The agent counts as installed when this exists.
    pub home: PathBuf,
    pub rules: PathBuf,
    pub rules_mode: RulesMode,
    pub skills: PathBuf,
    pub skills_mode: SkillsMode,
}

impl Agent {
    pub fn installed(&self) -> bool {
        self.home.exists()
    }

    /// Whether canonize acts on it at all.
    pub fn active(&self) -> bool {
        self.enabled && self.installed()
    }
}

pub struct Config {
    pub source: Source,
    pub agents: Vec<Agent>,
    /// Folders to look through for projects with a `CLAUDE.md` or `AGENTS.md`.
    pub projects: Vec<PathBuf>,
    /// The file the config was read from, or where it would be.
    pub path: PathBuf,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    projects: Vec<String>,
    #[serde(default)]
    source: RawSource,
    #[serde(default)]
    agents: BTreeMap<String, RawAgent>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawSource {
    rules: Option<String>,
    schema: Option<String>,
    house: Option<String>,
    skills: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAgent {
    enabled: Option<bool>,
    home: Option<String>,
    rules: Option<String>,
    rules_mode: Option<RulesMode>,
    skills: Option<String>,
    skills_mode: Option<SkillsMode>,
}

/// The agents canonize knows without being told, in display order.
fn defaults() -> Vec<Agent> {
    let agent = |name: &str, home, rules, rules_mode, skills, skills_mode| Agent {
        name: name.to_string(),
        enabled: true,
        home: expand(home),
        rules: expand(rules),
        rules_mode,
        skills: expand(skills),
        skills_mode,
    };
    vec![
        agent(
            "claude",
            "~/.claude",
            "~/.claude/CLAUDE.md",
            RulesMode::Import,
            "~/.claude/skills",
            SkillsMode::PerSkill,
        ),
        agent(
            "codex",
            "~/.codex",
            "~/.codex/AGENTS.md",
            RulesMode::Link,
            "~/.agents/skills",
            SkillsMode::Folder,
        ),
        agent(
            "pi",
            "~/.pi/agent",
            "~/.pi/agent/AGENTS.md",
            RulesMode::Link,
            "~/.agents/skills",
            SkillsMode::Folder,
        ),
        agent(
            "opencode",
            "~/.config/opencode",
            "~/.config/opencode/AGENTS.md",
            RulesMode::Link,
            "~/.config/opencode/skills",
            SkillsMode::PerSkill,
        ),
    ]
}

/// The source folder: `$CANONIZE_SOURCE` when set, else `~/.config/canonize`,
/// which a user can make a symlink to wherever their dotfiles keep it.
pub fn source_dir() -> PathBuf {
    match std::env::var_os("CANONIZE_SOURCE") {
        Some(dir) if !dir.is_empty() => expand(&dir.to_string_lossy()),
        _ => dirs::config_dir()
            .unwrap_or_else(|| home().join(".config"))
            .join("canonize"),
    }
}

pub fn load() -> Result<Config> {
    load_from(&source_dir())
}

pub fn load_from(root: &Path) -> Result<Config> {
    // A source reached through a link (`~/.config/canonize` pointing into the
    // dotfiles) is used by its real path, or every link and import canonize
    // writes would name the link and never match what the agents already read.
    let real = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let root = real.as_path();
    let path = root.join(CONFIG_FILE);
    if !path.exists() {
        bail!(
            "no `{}` in `{}`: run `canon` to set it up, or set `CANONIZE_SOURCE` to your canon folder",
            CONFIG_FILE,
            tilde(root)
        );
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("could not read `{}`", tilde(&path)))?;
    let raw: RawConfig =
        toml::from_str(&text).with_context(|| format!("`{}` is not valid", tilde(&path)))?;
    build(root, raw, path)
}

fn build(root: &Path, raw: RawConfig, path: PathBuf) -> Result<Config> {
    // An empty value turns the file off, so a source can go without a schema.
    let rel = |value: Option<String>, default: &str| {
        let value = value.unwrap_or_else(|| default.to_string());
        if value.is_empty() {
            PathBuf::new()
        } else if value.starts_with('~') || value.starts_with('/') {
            expand(&value)
        } else {
            root.join(value)
        }
    };
    let source = Source {
        root: root.to_path_buf(),
        rules: rel(raw.source.rules, "rules.yaml"),
        schema: rel(raw.source.schema, "rules.schema.json"),
        house: raw.source.house.unwrap_or_else(|| "house/*.md".to_string()),
        skills: rel(raw.source.skills, "skills"),
    };

    let mut agents = defaults();
    for (name, over) in raw.agents {
        let idx = match agents.iter().position(|a| a.name == name) {
            Some(i) => i,
            None => {
                let (Some(home), Some(rules), Some(skills)) =
                    (&over.home, &over.rules, &over.skills)
                else {
                    bail!(
                        "agent `{name}` in `{}` is not one canonize knows, so it needs `home`, `rules` and `skills`",
                        tilde(&path)
                    );
                };
                agents.push(Agent {
                    name: name.clone(),
                    enabled: true,
                    home: expand(home),
                    rules: expand(rules),
                    rules_mode: RulesMode::Link,
                    skills: expand(skills),
                    skills_mode: SkillsMode::Folder,
                });
                agents.len() - 1
            }
        };
        let a = &mut agents[idx];
        if let Some(v) = over.enabled {
            a.enabled = v;
        }
        if let Some(v) = over.home {
            a.home = expand(&v);
        }
        if let Some(v) = over.rules {
            a.rules = expand(&v);
        }
        if let Some(v) = over.rules_mode {
            a.rules_mode = v;
        }
        if let Some(v) = over.skills {
            a.skills = expand(&v);
        }
        if let Some(v) = over.skills_mode {
            a.skills_mode = v;
        }
    }
    Ok(Config {
        source,
        agents,
        projects: raw.projects.iter().map(|p| expand(p)).collect(),
        path,
    })
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/x` to an absolute path; anything else unchanged.
pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

/// An absolute path under the home folder written back as `~/…`, which is how
/// every path is shown and how an import line is written.
pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The house files the pattern matches, sorted by name.
pub fn house_files(source: &Source) -> Vec<PathBuf> {
    if source.house.is_empty() {
        return Vec::new();
    }
    let pattern = source.root.join(&source.house);
    let (Some(dir), Some(name)) = (pattern.parent(), pattern.file_name()) else {
        return Vec::new();
    };
    let name = name.to_string_lossy();
    let (pre, post) = name.split_once('*').unwrap_or((&name, ""));
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().into_owned());
                n.is_some_and(|n| {
                    if name.contains('*') {
                        n.len() >= pre.len() + post.len() && n.starts_with(pre) && n.ends_with(post)
                    } else {
                        n == *name
                    }
                })
            })
            .filter(|p| p.is_file())
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
}

/// The skill folders in the source, sorted by name.
pub fn skill_names(source: &Source) -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(&source.skills) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}
