//! First run: find the rules a user already feeds their agents, and set the
//! source up around them, so nobody starts from an empty screen.

use crate::config::{self, CONFIG_FILE, tilde};
use anyhow::{Context, Result, bail};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// A source worked out from what the agents already read.
pub struct Found {
    pub root: PathBuf,
    pub rules: PathBuf,
    pub schema: Option<PathBuf>,
    /// The house pattern, when the folder has house files.
    pub house: Option<String>,
    /// Which agent file pointed at it, for the user to recognise.
    pub by: PathBuf,
}

impl Found {
    /// What setup would choose with no questions asked, for `canon setup`.
    pub fn choice(&self) -> Choice {
        let name = |p: &Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        Choice {
            root: self.root.clone(),
            rules: Pick::Existing(name(&self.rules)),
            schema: self
                .schema
                .as_ref()
                .map_or(Pick::None, |s| Pick::Existing(name(s))),
            house: self.house.clone(),
            skills: "skills".to_string(),
            projects: crate::projects::guess(),
        }
    }
}

/// One answer in the setup wizard: a file that is there, one to create from
/// the starter, or none.
#[derive(Clone, PartialEq, Eq)]
pub enum Pick {
    Existing(String),
    New,
    None,
}

/// Everything setup needs, as the user answered it.
pub struct Choice {
    pub root: PathBuf,
    pub rules: Pick,
    pub schema: Pick,
    pub house: Option<String>,
    /// Relative to `root`.
    pub skills: String,
    /// Folders to look through for projects, as the user wrote them.
    pub projects: Vec<String>,
}

impl Choice {
    /// The config it writes, each line only where it differs from the default.
    pub fn config_text(&self) -> String {
        let q = |p: &Pick, default: &str| match p {
            Pick::Existing(n) => n.clone(),
            Pick::New => default.to_string(),
            Pick::None => String::new(),
        };
        let list: Vec<String> = self.projects.iter().map(|p| format!("\"{p}\"")).collect();
        crate::init::config_text(
            &format!("[{}]", list.join(", ")),
            &q(&self.rules, "rules.yaml"),
            &q(&self.schema, "rules.schema.json"),
            self.house.as_deref().unwrap_or(""),
            &self.skills,
        )
    }

    /// The answers and their consequences in plain sentences, for the last
    /// box before anything is written.
    pub fn summary(&self) -> Vec<String> {
        let mut out = vec![format!("Your canon lives in {}.", tilde(&self.root))];
        out.push(match &self.rules {
            Pick::Existing(n) => format!("Your rules are {n}."),
            Pick::New => "Your rules start as a new rules.yaml from the starter.".to_string(),
            Pick::None => "You have no rules file, so each agent keeps its own.".to_string(),
        });
        out.push(match (&self.rules, &self.schema) {
            (Pick::None, _) => {
                "There is nothing for canon validate to check them against.".to_string()
            }
            (_, Pick::Existing(n)) => format!("canon validate holds them to {n}."),
            (_, Pick::New) => "canon validate holds them to a new rules.schema.json.".to_string(),
            (_, Pick::None) => {
                "They have no schema, so canon validate only looks at key order.".to_string()
            }
        });
        out.push(match &self.house {
            Some(h) => format!("Your house files are {h}."),
            None => "You have no house files.".to_string(),
        });
        out.push(format!(
            "Your skills live in {}/.",
            tilde(&self.root.join(&self.skills))
        ));
        out.push(if self.projects.is_empty() {
            "It looks for no projects.".to_string()
        } else {
            format!("It looks for projects in {}.", self.projects.join(", "))
        });
        out.push(String::new());
        let mut writes = vec![format!("canonize.toml in {}", tilde(&self.root))];
        if self.rules == Pick::New {
            writes.push("rules.yaml".to_string());
        }
        if self.schema == Pick::New {
            writes.push("rules.schema.json".to_string());
        }
        out.push(format!("It writes {}.", writes.join(", ")));
        if let Some(link) = pointer_link(&self.root) {
            out.push(format!(
                "It adds the shortcut {} to your canon, so `canon` finds it next time.",
                tilde(&link)
            ));
        }
        out.push("Nothing else changes.".to_string());
        out
    }
}

/// The default source path, when it has to become a link for `canon` to find
/// `root` with no environment variable: `None` when nothing needs doing.
fn pointer_link(root: &Path) -> Option<PathBuf> {
    if std::env::var_os("CANONIZE_SOURCE").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let default = config::source_dir();
    if default == root || default.exists() || fs::symlink_metadata(&default).is_ok() {
        return None;
    }
    Some(default)
}

/// Look through the agents' instruction files for a rules file already in use:
/// an `@path` import, or the file itself being a link.
pub fn detect() -> Option<Found> {
    let agents = [
        "~/.claude/CLAUDE.md",
        "~/.codex/AGENTS.md",
        "~/.pi/agent/AGENTS.md",
        "~/.config/opencode/AGENTS.md",
    ];
    for file in agents.map(config::expand) {
        for rules in candidates(&file) {
            if let Some(found) = around(&rules, &file) {
                return Some(found);
            }
        }
    }
    None
}

/// What the wizard and `canon setup` both start from: the rules an agent
/// reads, or the ones it read before the canon moved and that turned up in a
/// folder next door.
pub fn detect_or_moved() -> Option<Found> {
    detect().or_else(|| {
        let l = lost()?;
        around(&l.now?, &l.by)
    })
}

/// An import an agent still has of a rules file that is gone, and where a
/// file by that name sits now, when a nearby folder has one.
pub struct Lost {
    pub by: PathBuf,
    pub path: PathBuf,
    pub now: Option<PathBuf>,
}

/// The first rules import that names a missing file, for when `detect` found
/// nothing: the canon was most likely moved or renamed.
pub fn lost() -> Option<Lost> {
    let agents = [
        "~/.claude/CLAUDE.md",
        "~/.codex/AGENTS.md",
        "~/.pi/agent/AGENTS.md",
        "~/.config/opencode/AGENTS.md",
    ];
    for by in agents.map(config::expand) {
        for path in candidates(&by) {
            let name = path.file_name().map(|n| n.to_string_lossy().to_uppercase());
            if path.exists() || name.is_none_or(|n| n.starts_with("HOUSE")) {
                continue;
            }
            let now = nearby(&path);
            return Some(Lost { by, path, now });
        }
    }
    None
}

/// A file with `gone`'s name in a sibling of its old folder, or one level
/// below the folder above that: where a renamed or moved canon ends up.
fn nearby(gone: &Path) -> Option<PathBuf> {
    let name = gone.file_name()?;
    let old_dir = gone.parent()?;
    let base = old_dir.parent()?;
    let mut dirs: Vec<PathBuf> = fs::read_dir(base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p != old_dir)
        .collect();
    dirs.sort();
    dirs.into_iter().map(|d| d.join(name)).find(|f| f.is_file())
}

fn candidates(file: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dest) = fs::read_link(file) {
        out.push(dest);
    }
    if let Ok(text) = fs::read_to_string(file) {
        out.extend(
            text.lines()
                .filter_map(|l| l.trim().strip_prefix('@'))
                .filter(|p| !p.contains(' '))
                .map(config::expand),
        );
    }
    out
}

pub fn around(rules: &Path, by: &Path) -> Option<Found> {
    let name = rules.file_name()?.to_string_lossy().into_owned();
    // A house file imported globally names the folder, but it is not the rules.
    if !rules.is_file() || name.to_uppercase().starts_with("HOUSE") {
        return None;
    }
    let root = rules.parent()?.to_path_buf();
    let stem = rules.file_stem()?.to_string_lossy().into_owned();
    let schema = [
        format!("{stem}.schema.json"),
        "rules.schema.json".to_string(),
    ]
    .into_iter()
    .map(|n| root.join(n))
    .find(|p| p.is_file());
    let has = |pattern: &str| {
        let probe = config::Source {
            root: root.clone(),
            rules: rules.to_path_buf(),
            schema: PathBuf::new(),
            house: pattern.to_string(),
            skills: PathBuf::new(),
        };
        !config::house_files(&probe).is_empty()
    };
    let house = ["house/*.md", "HOUSE-*.md"]
        .into_iter()
        .find(|p| has(p))
        .map(str::to_string);
    Some(Found {
        root,
        rules: rules.to_path_buf(),
        schema,
        house,
        by: by.to_path_buf(),
    })
}

/// Write the config, any starter files the answers asked for, and the link
/// that lets `canon` find the folder. Refuses to overwrite a config.
pub fn apply(choice: &Choice) -> Result<()> {
    let root = &choice.root;
    fs::create_dir_all(root).with_context(|| format!("could not create `{}`", tilde(root)))?;
    let cfg = root.join(CONFIG_FILE);
    if cfg.exists() {
        bail!("`{}` already exists, so it was left alone", tilde(&cfg));
    }
    let starter = |name: &str, body: &str| -> Result<()> {
        let path = root.join(name);
        if !path.exists() {
            fs::write(&path, body)
                .with_context(|| format!("could not write `{}`", tilde(&path)))?;
        }
        Ok(())
    };
    if choice.rules == Pick::New {
        starter("rules.yaml", crate::init::RULES)?;
    }
    if choice.schema == Pick::New {
        starter("rules.schema.json", crate::init::SCHEMA)?;
    }
    fs::write(&cfg, choice.config_text())
        .with_context(|| format!("could not write `{}`", tilde(&cfg)))?;
    if let Some(link) = pointer_link(root) {
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create `{}`", tilde(parent)))?;
        }
        symlink(root, &link).with_context(|| format!("could not link `{}`", tilde(&link)))?;
    }
    Ok(())
}

/// Files in `root` that could be a rules file, sorted.
pub fn rule_files(root: &Path) -> Vec<String> {
    files(root, |n| {
        let low = n.to_lowercase();
        (low.ends_with(".yaml") || low.ends_with(".yml") || low.ends_with(".md"))
            && !low.starts_with("house")
            && !low.starts_with("readme")
    })
}

/// Files in `root` that look like a JSON schema, sorted.
pub fn schema_files(root: &Path) -> Vec<String> {
    files(root, |n| n.to_lowercase().ends_with(".schema.json"))
}

/// The house patterns that match something in `root`, with how many files.
pub fn house_patterns(root: &Path) -> Vec<(String, usize)> {
    ["HOUSE-*.md", "house/*.md"]
        .into_iter()
        .filter_map(|p| {
            let probe = config::Source {
                root: root.to_path_buf(),
                rules: PathBuf::new(),
                schema: PathBuf::new(),
                house: p.to_string(),
                skills: PathBuf::new(),
            };
            let n = config::house_files(&probe).len();
            (n > 0).then(|| (p.to_string(), n))
        })
        .collect()
}

fn files(root: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| keep(n))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Move an agent's own skill folder into the source and leave a link in its
/// place, so the agent sees no difference and the skill is now managed.
pub fn adopt(from: &Path, skills: &Path) -> Result<PathBuf> {
    let name = from
        .file_name()
        .with_context(|| format!("`{}` has no name", tilde(from)))?;
    let to = skills.join(name);
    if fs::symlink_metadata(from).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("`{}` is a link, not a skill folder", tilde(from));
    }
    if to.exists() {
        bail!("`{}` already exists in the source", tilde(&to));
    }
    fs::create_dir_all(skills).with_context(|| format!("could not create `{}`", tilde(skills)))?;
    fs::rename(from, &to).with_context(|| {
        format!(
            "could not move `{}` into `{}` (a different disk needs a manual move)",
            tilde(from),
            tilde(skills)
        )
    })?;
    symlink(&to, from).with_context(|| format!("could not link `{}`", tilde(from)))?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmp::Temp;

    #[test]
    fn a_canon_is_recognised_by_the_rules_file_an_agent_already_reads() {
        let t = Temp::new();
        let rules = t.write("canon/rules.yaml", "title: MYRULES\n");
        t.write("canon/rules.schema.json", "{}");
        t.write("canon/house/HOUSE-RUST.md", "");
        let by = t.write("claude/CLAUDE.md", "");

        let found = around(&rules, &by).expect("the rules file should be recognised");
        assert_eq!(found.root, t.at("canon"));
        assert_eq!(found.schema, Some(t.at("canon/rules.schema.json")));
        assert_eq!(found.house.as_deref(), Some("house/*.md"));
    }

    #[test]
    fn a_schema_named_after_the_rules_file_is_found_too() {
        let t = Temp::new();
        let rules = t.write("canon/MYRULES.yaml", "title: MYRULES\n");
        t.write("canon/MYRULES.schema.json", "{}");
        let by = t.write("claude/CLAUDE.md", "");

        let found = around(&rules, &by).expect("the rules file should be recognised");
        assert_eq!(found.schema, Some(t.at("canon/MYRULES.schema.json")));
        assert_eq!(found.house, None, "there are no house files to find");
    }

    #[test]
    fn a_house_file_imported_on_its_own_is_never_taken_for_the_rules() {
        let t = Temp::new();
        let house = t.write("canon/HOUSE-RUST.md", "");
        let by = t.write("claude/CLAUDE.md", "");
        assert!(around(&house, &by).is_none());
        assert!(
            around(&t.at("canon/gone.yaml"), &by).is_none(),
            "a file that is not there is not a canon"
        );
    }

    #[test]
    fn the_house_patterns_offered_are_the_ones_that_match_something() {
        let t = Temp::new();
        t.write("canon/HOUSE-RUST.md", "");
        t.write("canon/HOUSE-TUI.md", "");
        assert_eq!(
            house_patterns(&t.at("canon")),
            [("HOUSE-*.md".to_string(), 2)]
        );
    }

    #[test]
    fn the_files_offered_as_rules_leave_out_house_files_and_readmes() {
        let t = Temp::new();
        t.write("canon/rules.yaml", "");
        t.write("canon/MYRULES.md", "");
        t.write("canon/HOUSE-RUST.md", "");
        t.write("canon/README.md", "");
        t.write("canon/rules.schema.json", "");
        assert_eq!(rule_files(&t.at("canon")), ["MYRULES.md", "rules.yaml"]);
        assert_eq!(schema_files(&t.at("canon")), ["rules.schema.json"]);
    }

    #[test]
    fn setup_never_writes_over_a_config_that_is_already_there() {
        let t = Temp::new();
        let root = t.dir("canon");
        let mine = "projects = []\n";
        fs::write(root.join(CONFIG_FILE), mine).expect("could not write");
        let choice = Choice {
            root: root.clone(),
            rules: Pick::New,
            schema: Pick::None,
            house: None,
            skills: "skills".to_string(),
            projects: Vec::new(),
        };

        assert!(
            apply(&choice).is_err(),
            "setup must refuse a canon it did not write"
        );
        assert_eq!(
            fs::read_to_string(root.join(CONFIG_FILE)).expect("the config should still be there"),
            mine
        );
        assert!(
            !root.join("rules.yaml").exists(),
            "nothing else is written once it refuses"
        );
    }

    #[test]
    fn the_config_setup_writes_reads_back_as_the_answers_given() {
        let t = Temp::new();
        let root = t.dir("canon");
        let choice = Choice {
            root: root.clone(),
            rules: Pick::Existing("MYRULES.md".to_string()),
            schema: Pick::None,
            house: Some("HOUSE-*.md".to_string()),
            skills: "skills".to_string(),
            projects: vec!["~/dev".to_string(), "~/work".to_string()],
        };
        fs::write(root.join(CONFIG_FILE), choice.config_text()).expect("could not write");

        let cfg = config::load_from(&root).expect("the config setup writes has to load");
        assert_eq!(cfg.source.rules, cfg.source.root.join("MYRULES.md"));
        assert!(
            cfg.source.schema.as_os_str().is_empty(),
            "no schema was chosen"
        );
        assert_eq!(cfg.source.house, "HOUSE-*.md");
        assert_eq!(
            cfg.projects,
            [config::expand("~/dev"), config::expand("~/work")]
        );
    }

    #[test]
    fn adopting_a_skill_moves_the_whole_folder_and_leaves_a_link_in_its_place() {
        let t = Temp::new();
        let from = t.skill("claude/skills/audit", "audit");
        t.write("claude/skills/audit/run.sh", "echo hi\n");
        let skills = t.at("canon/skills");

        let to = adopt(&from, &skills).expect("the skill should move");
        assert_eq!(to, skills.join("audit"));
        assert_eq!(
            fs::read_link(&from).expect("a link should be left in its place"),
            to
        );
        assert_eq!(
            fs::read_to_string(from.join("run.sh")).expect("the agent should see no difference"),
            "echo hi\n",
            "scripts move with the skill"
        );
    }

    #[test]
    fn adopting_refuses_a_link_and_a_name_the_canon_already_has() {
        let t = Temp::new();
        let from = t.skill("claude/skills/audit", "audit");
        let skills = t.dir("canon/skills");
        t.skill("canon/skills/audit", "audit");
        assert!(
            adopt(&from, &skills).is_err(),
            "a skill the canon already has would be overwritten"
        );

        let linked = t.at("claude/skills/rust");
        symlink(t.skill("canon/skills/rust", "rust"), &linked).expect("could not link");
        assert!(
            adopt(&linked, &skills).is_err(),
            "a link is already managed, so there is nothing to move"
        );
    }
}
