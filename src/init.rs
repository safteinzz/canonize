//! `canon init`: lay out a new source folder in the standard shape.

use crate::config::{CONFIG_FILE, tilde};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub const RULES: &str = include_str!("../templates/rules.yaml");
pub const SCHEMA: &str = include_str!("../templates/rules.schema.json");
pub const WORKSPACES: &str = include_str!("../templates/workspaces.md");

const FILES: [(&str, &str); 3] = [
    ("rules.yaml", RULES),
    ("rules.schema.json", SCHEMA),
    (crate::config::WORKSPACE_RULES, WORKSPACES),
];

/// canonize.toml with every setting written out and explained, for someone
/// who has never seen TOML: setup fills in the answers, init the defaults.
/// What fills the template, each as it is written in the file.
pub struct Answers<'a> {
    /// A TOML list of the folders to look through.
    pub scopes: &'a str,
    /// A TOML list of the folders named as projects.
    pub projects: &'a str,
    pub rules: &'a str,
    pub schema: &'a str,
    pub house: &'a str,
    pub skills: &'a str,
    pub workspaces: &'a str,
}

pub fn config_text(a: &Answers) -> String {
    let Answers {
        scopes,
        projects,
        rules,
        schema,
        house,
        skills,
        workspaces,
    } = a;
    include_str!("../templates/canonize.toml")
        .replace("{scopes}", scopes)
        .replace("{projects}", projects)
        .replace("{workspaces}", workspaces)
        .replace("{rules}", rules)
        .replace("{schema}", schema)
        .replace("{house}", house)
        .replace("{skills}", skills)
}

const DIRS: [&str; 2] = ["conventions", "skills"];

/// Create what is missing and report each path with whether it was created.
/// An existing file is never overwritten, so running it twice is harmless. A
/// dry run reports the same list and writes nothing.
pub fn run(root: &Path, dry_run: bool) -> Result<Vec<(String, bool)>> {
    if !dry_run {
        fs::create_dir_all(root).with_context(|| format!("could not create `{}`", tilde(root)))?;
    }
    let mut done = Vec::new();
    let config = config_text(&Answers {
        scopes: "[]",
        projects: "[]",
        rules: "rules.yaml",
        schema: "rules.schema.json",
        house: "conventions/*.md",
        skills: "skills",
        workspaces: crate::config::WORKSPACE_RULES,
    });
    let files = std::iter::once((CONFIG_FILE, config.as_str())).chain(FILES);
    for (name, body) in files {
        let path = root.join(name);
        let created = !path.exists();
        if created && !dry_run {
            fs::write(&path, body)
                .with_context(|| format!("could not write `{}`", tilde(&path)))?;
        }
        done.push((tilde(&path), created));
    }
    for name in DIRS {
        let path = root.join(name);
        let created = !path.exists();
        if !dry_run {
            fs::create_dir_all(&path)
                .with_context(|| format!("could not create `{}`", tilde(&path)))?;
        }
        done.push((format!("{}/", tilde(&path)), created));
    }
    Ok(done)
}
