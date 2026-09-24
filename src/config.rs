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
    /// The folder `canon` looks in, which is a link to the source when the
    /// source lives somewhere else.
    pub shortcut: PathBuf,
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
            "no `{}` in `{}`: run `canon setup` to set it up around the rules your agents already read, `canon init` to start a new canon, or set `CANONIZE_SOURCE` to the folder yours is in",
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
        shortcut: source_dir(),
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

/// The skills in the source, sorted by name: a folder with a `SKILL.md`, which
/// is what an agent can load.
pub fn skill_names(source: &Source) -> Vec<String> {
    let mut names: Vec<String> = skill_dirs(source)
        .into_iter()
        .filter(|(_, is_skill)| *is_skill)
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names
}

/// Folders in the skills folder that are not skills, each with the skills it
/// holds inside: an agent whose skills folder is the canon's writes its own
/// tree there (Claude's `synced` bucket), and none of it can be loaded.
pub fn strays(source: &Source) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = skill_dirs(source)
        .into_iter()
        .filter(|(_, is_skill)| !*is_skill)
        .map(|(name, _)| {
            let mut inside = Vec::new();
            skills_inside(&source.skills.join(&name), 3, &mut inside);
            inside.sort();
            (name, inside)
        })
        .collect();
    out.sort();
    out
}

/// Every folder in the skills folder, with whether it is a skill.
fn skill_dirs(source: &Source) -> Vec<(String, bool)> {
    match fs::read_dir(&source.skills) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| {
                let is_skill = e.path().join("SKILL.md").is_file();
                (e.file_name().to_string_lossy().into_owned(), is_skill)
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The names of the skills anywhere under `dir`, down to `depth` folders.
fn skills_inside(dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten().filter(|e| e.path().is_dir()) {
        let path = e.path();
        if path.join("SKILL.md").is_file() {
            out.push(e.file_name().to_string_lossy().into_owned());
        } else {
            skills_inside(&path, depth - 1, out);
        }
    }
}

/// Set one key in `canonize.toml` by rewriting its own line, so every comment
/// the file carries survives. Returns the line as it now reads, and leaves the
/// file exactly as it was when the result would not load.
pub fn set_key(path: &Path, key: &str, literal: &str, dry_run: bool) -> Result<String> {
    let text =
        fs::read_to_string(path).with_context(|| format!("could not read `{}`", tilde(path)))?;
    let (section, leaf) = match key.rsplit_once('.') {
        Some((section, leaf)) => (Some(section), leaf),
        None => (None, key),
    };
    if leaf.is_empty() || leaf.contains(char::is_whitespace) {
        bail!(
            "`{key}` is not a key: write it as `projects`, `source.rules` or `agents.pi.skills_mode`"
        );
    }

    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut here: Option<String> = None;
    let mut found = None;
    let mut section_at = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            here = Some(name.to_string());
            if Some(name) == section {
                section_at = Some(i);
            }
            continue;
        }
        if here.as_deref() == section
            && trimmed
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == leaf)
        {
            found = Some(i);
            break;
        }
    }

    // The line, keeping the padding the file already uses around `=`.
    let line = match found {
        Some(i) => {
            let head = lines[i]
                .split_once('=')
                .map_or(leaf.to_string(), |(h, _)| h.to_string());
            format!("{head}= {literal}")
        }
        None => format!("{leaf} = {literal}"),
    };
    match (found, section_at, section) {
        (Some(i), _, _) => lines[i] = line.clone(),
        (None, Some(at), _) => lines.insert(at + 1, line.clone()),
        (None, None, Some(section)) => {
            if !lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(line.clone());
        }
        (None, None, None) => lines.push(line.clone()),
    }

    if dry_run {
        return Ok(line);
    }
    let mut body = lines.join("\n");
    body.push('\n');
    fs::write(path, &body).with_context(|| format!("could not write `{}`", tilde(path)))?;
    // A config that no longer loads is worse than the setting being unset, so
    // the file goes back exactly as it was and the parser's words are the error.
    let root = path.parent().unwrap_or(Path::new("."));
    if let Err(e) = load_from(root) {
        fs::write(path, &text).with_context(|| format!("could not write `{}`", tilde(path)))?;
        bail!("`{key} = {literal}` was not kept, since the config then reads: {e:#}");
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmp::{Temp, source};

    fn names(files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|f| {
                f.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn a_path_written_with_a_tilde_expands_back_to_the_same_path() {
        let inside = home().join("dotfiles/canon/rules.yaml");
        assert_eq!(
            expand(&tilde(&inside)),
            inside,
            "an import line is written with `tilde` and read back with `expand`, so an agent's import would name another file"
        );
        let outside = PathBuf::from("/etc/hosts");
        assert_eq!(expand(&tilde(&outside)), outside);
        assert_eq!(expand(&tilde(&home())), home());
    }

    #[test]
    fn the_house_pattern_takes_files_by_prefix_and_suffix_and_nothing_else() {
        let t = Temp::new();
        t.write("HOUSE-RUST.md", "");
        t.write("HOUSE-TUI.md", "");
        t.write("README.md", "");
        t.write("HOUSE-RUST.md.bak", "");
        t.dir("HOUSE-FOLDER.md");
        let mut s = source(t.path());
        s.house = "HOUSE-*.md".to_string();
        assert_eq!(names(&house_files(&s)), ["HOUSE-RUST.md", "HOUSE-TUI.md"]);
    }

    #[test]
    fn a_pattern_with_no_star_names_exactly_one_file() {
        let t = Temp::new();
        t.write("AGENTS.md", "");
        t.write("AGENTS.md.old", "");
        let mut s = source(t.path());
        s.house = "AGENTS.md".to_string();
        assert_eq!(names(&house_files(&s)), ["AGENTS.md"]);
    }

    #[test]
    fn an_empty_name_in_the_config_turns_that_file_off() {
        let t = Temp::new();
        t.write("rules.yaml", "title: x\n");
        t.write("house/HOUSE-RUST.md", "");
        t.write(
            CONFIG_FILE,
            "[source]\nrules = \"\"\nschema = \"\"\nhouse = \"\"\n",
        );
        let cfg = load_from(t.path()).expect("the config should load");
        assert!(
            cfg.source.rules.as_os_str().is_empty(),
            "an empty `rules` means the user keeps their rules in each agent's own file"
        );
        assert!(cfg.source.schema.as_os_str().is_empty());
        assert!(house_files(&cfg.source).is_empty());
    }

    #[test]
    fn a_name_in_the_config_is_read_inside_the_canon_unless_it_is_absolute() {
        let t = Temp::new();
        t.write(
            CONFIG_FILE,
            "[source]\nrules = \"MYRULES.md\"\nskills = \"/opt/skills\"\n",
        );
        let cfg = load_from(t.path()).expect("the config should load");
        assert_eq!(cfg.source.rules, cfg.source.root.join("MYRULES.md"));
        assert_eq!(cfg.source.skills, PathBuf::from("/opt/skills"));
    }

    #[test]
    fn an_agent_canonize_does_not_know_needs_home_rules_and_skills() {
        let t = Temp::new();
        t.write(CONFIG_FILE, "[agents.zed]\nenabled = true\n");
        assert!(
            load_from(t.path()).is_err(),
            "an unknown agent with no paths has nowhere to link anything"
        );

        let t = Temp::new();
        t.write(
            CONFIG_FILE,
            "[agents.zed]\nhome = \"/tmp/zed\"\nrules = \"/tmp/zed/RULES.md\"\nskills = \"/tmp/zed/skills\"\n",
        );
        let cfg = load_from(t.path()).expect("an unknown agent with all three should load");
        let zed = cfg
            .agents
            .iter()
            .find(|a| a.name == "zed")
            .expect("zed should be an agent");
        assert_eq!(zed.home, PathBuf::from("/tmp/zed"));
    }

    #[test]
    fn a_config_key_that_is_not_one_of_ours_is_an_error() {
        let t = Temp::new();
        t.write(CONFIG_FILE, "[source]\nrulez = \"rules.yaml\"\n");
        assert!(
            load_from(t.path()).is_err(),
            "a misspelled key must be an error, or it silently does nothing"
        );
    }

    /// The config as `canon init` writes it: every setting, each with a comment.
    fn written_config(t: &Temp) -> PathBuf {
        t.write(
            CONFIG_FILE,
            &crate::init::config_text(
                "[]",
                "rules.yaml",
                "rules.schema.json",
                "house/*.md",
                "skills",
            ),
        )
    }

    #[test]
    fn a_setting_is_written_on_its_own_line_and_every_comment_stays() {
        let t = Temp::new();
        let path = written_config(&t);
        let before = fs::read_to_string(&path).expect("it should be there");
        let comments = before
            .lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .count();

        set_key(&path, "source.rules", "\"MYRULES.md\"", false).expect("it should write");
        let after = fs::read_to_string(&path).expect("it should be there");
        assert_eq!(
            after
                .lines()
                .filter(|l| l.trim_start().starts_with('#'))
                .count(),
            comments,
            "the file explains itself, so its comments outlive any setting"
        );
        let cfg = load_from(t.path()).expect("it should still load");
        assert_eq!(cfg.source.rules, cfg.source.root.join("MYRULES.md"));
        assert_eq!(
            after
                .lines()
                .filter(|l| l.trim_start().starts_with("rules"))
                .count(),
            1,
            "the setting is rewritten, never added beside itself"
        );
    }

    #[test]
    fn a_setting_with_no_line_yet_is_added_under_its_section() {
        let t = Temp::new();
        let path = written_config(&t);
        set_key(&path, "agents.pi.skills_mode", "\"per-skill\"", false).expect("a new section");
        set_key(&path, "agents.pi.rules_mode", "\"import\"", false).expect("a new line in it");

        let cfg = load_from(t.path()).expect("it should load");
        let pi = cfg
            .agents
            .iter()
            .find(|a| a.name == "pi")
            .expect("pi is known");
        assert_eq!(pi.skills_mode, SkillsMode::PerSkill);
        assert_eq!(pi.rules_mode, RulesMode::Import);
    }

    #[test]
    fn a_setting_that_would_not_load_leaves_the_file_exactly_as_it_was() {
        let t = Temp::new();
        let path = written_config(&t);
        let before = fs::read_to_string(&path).expect("it should be there");

        let err = set_key(&path, "agents.pi.skills_mode", "\"per-skil\"", false)
            .expect_err("`per-skil` is not a mode");
        assert!(
            format!("{err:#}").contains("per-skill"),
            "the parser's own words say what is allowed: {err:#}"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("it should be there"),
            before,
            "a config that no longer loads is worse than the setting being unset"
        );
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let t = Temp::new();
        let path = written_config(&t);
        let before = fs::read_to_string(&path).expect("it should be there");
        let line = set_key(&path, "projects", "[\"~/dev\"]", true).expect("it should plan");
        assert!(line.contains("~/dev"), "{line}");
        assert_eq!(
            fs::read_to_string(&path).expect("it should be there"),
            before
        );
    }

    #[test]
    fn only_folders_are_skills_and_hidden_ones_are_not() {
        let t = Temp::new();
        t.skill("skills/rust", "rust");
        t.skill("skills/audit", "audit");
        t.write("skills/notes.md", "");
        t.dir("skills/.git");
        assert_eq!(skill_names(&source(t.path())), ["audit", "rust"]);
    }
}
