//! Scaffolding the tests share: a temp folder that deletes itself, and the
//! structs the engines take, built by hand.
//!
//! The crate is a binary with no library target, so a test cannot import it;
//! every test lives beside the code it covers and gets its files from here.

use crate::config::{Agent, Config, RulesMode, SkillsMode, Source};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A folder under the system temp dir, deleted when the test ends. Nothing a
/// test writes ever lands in a real config, data or cache path.
pub struct Temp(PathBuf);

impl Temp {
    pub fn new() -> Temp {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canonize-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("could not create the test's temp folder");
        Temp(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn at(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// Write a file, creating the folders above it.
    pub fn write(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.at(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("could not create a test folder");
        }
        fs::write(&path, body).expect("could not write a test file");
        path
    }

    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.at(rel);
        fs::create_dir_all(&path).expect("could not create a test folder");
        path
    }

    /// A skill with the frontmatter `canon validate` asks for.
    pub fn skill(&self, rel: &str, name: &str) -> PathBuf {
        self.write(
            &format!("{rel}/SKILL.md"),
            &format!("---\nname: {name}\ndescription: what {name} does\n---\n\nbody\n"),
        );
        self.at(rel)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A canon laid out the way the defaults expect it.
pub fn source(root: &Path) -> Source {
    Source {
        root: root.to_path_buf(),
        rules: root.join("rules.yaml"),
        schema: root.join("rules.schema.json"),
        house: "house/*.md".to_string(),
        skills: root.join("skills"),
    }
}

pub fn agent(
    name: &str,
    home: &Path,
    rules: &Path,
    rules_mode: RulesMode,
    skills: &Path,
    skills_mode: SkillsMode,
) -> Agent {
    Agent {
        name: name.to_string(),
        enabled: true,
        home: home.to_path_buf(),
        rules: rules.to_path_buf(),
        rules_mode,
        skills: skills.to_path_buf(),
        skills_mode,
    }
}

/// Claude's wiring: an `@` import for the rules, one link per skill.
pub fn claude(home: &Path) -> Agent {
    agent(
        "claude",
        home,
        &home.join("CLAUDE.md"),
        RulesMode::Import,
        &home.join("skills"),
        SkillsMode::PerSkill,
    )
}

pub fn config(source: Source, agents: Vec<Agent>, projects: Vec<PathBuf>) -> Config {
    let path = source.root.join(crate::config::CONFIG_FILE);
    let shortcut = source.root.join(".shortcut-that-is-not-there");
    Config {
        source,
        agents,
        projects,
        path,
        // A test never reads the real one: `~/.config/canonize` is a real path
        // on the machine running the suite.
        shortcut,
    }
}
