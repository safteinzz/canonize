//! Whether the source itself is sound: the rules file parses, matches its
//! schema and keeps its keys in the order its own `format.keys` lists, and
//! every skill has a `SKILL.md` an agent can read.

use crate::config::{self, Source, tilde};
use serde_yaml::Value;
use std::fs;

pub struct Report {
    pub problems: Vec<String>,
    /// How many rules the rules file holds, when it parsed.
    pub rules: Option<usize>,
    pub skills: usize,
    pub house: usize,
}

/// `48 rules · 6 skills · 3 house files`, each noun agreeing with its count.
pub fn counts(r: &Report) -> String {
    let n = |count: usize, one: &str| format!("{count} {one}{}", if count == 1 { "" } else { "s" });
    // A rules file that is not YAML, or none at all, has no count to give.
    let mut parts = Vec::new();
    if let Some(rules) = r.rules {
        parts.push(n(rules, "rule"));
    }
    parts.push(n(r.skills, "skill"));
    parts.push(n(r.house, "house file"));
    parts.join(" · ")
}

pub fn run(source: &Source) -> Report {
    let mut problems = Vec::new();
    let rules = check_rules(source, &mut problems);
    let skills = config::skill_names(source);
    for name in &skills {
        check_skill(source, name, &mut problems);
    }
    Report {
        problems,
        rules,
        skills: skills.len(),
        house: config::house_files(source).len(),
    }
}

fn check_rules(source: &Source, problems: &mut Vec<String>) -> Option<usize> {
    // No rules file: the user keeps their rules in each agent's own file.
    if source.rules.as_os_str().is_empty() {
        return None;
    }
    let file = tilde(&source.rules);
    let text = match fs::read_to_string(&source.rules) {
        Ok(t) => t,
        Err(_) => {
            problems.push(format!("{file}: missing, so no agent gets any rules"));
            return None;
        }
    };
    // Only a YAML rules file is held to the schema and the key order; any
    // other format is the user's own and only has to be there.
    let yaml = source
        .rules
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"));
    if !yaml {
        return None;
    }
    let doc: Value = match serde_yaml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("{file}: not valid YAML: {e}"));
            return None;
        }
    };

    if source.schema.exists() {
        check_schema(source, &doc, problems);
    }

    let order: Vec<String> = doc
        .get("format")
        .and_then(|f| f.get("keys"))
        .and_then(Value::as_mapping)
        .map(|m| {
            m.keys()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut count = 0;
    walk_rules(&doc, &mut |rule| {
        count += 1;
        if order.is_empty() {
            return;
        }
        let name = rule
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)");
        let keys: Vec<&str> = rule
            .as_mapping()
            .map(|m| m.keys().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let known: Vec<usize> = keys
            .iter()
            .filter_map(|k| order.iter().position(|o| o == k))
            .collect();
        if known.windows(2).any(|w| w[0] > w[1]) {
            problems.push(format!(
                "{file}: rule `{name}` has its keys out of the order `format.keys` lists"
            ));
        }
        for k in keys.iter().filter(|k| !order.iter().any(|o| o == *k)) {
            problems.push(format!(
                "{file}: rule `{name}` uses `{k}`, which `format.keys` does not define"
            ));
        }
    });
    Some(count)
}

/// Every mapping with both a `name` and a `rule` is a rule, wherever it sits,
/// so a file can group its rules however it likes.
fn walk_rules(v: &Value, f: &mut dyn FnMut(&Value)) {
    match v {
        Value::Mapping(m) => {
            if m.contains_key("name") && m.contains_key("rule") {
                f(v);
            }
            // `format.keys` describes the keys by name, so it looks like a rule.
            for (k, child) in m {
                if k.as_str() != Some("format") {
                    walk_rules(child, f);
                }
            }
        }
        Value::Sequence(s) => s.iter().for_each(|c| walk_rules(c, f)),
        _ => {}
    }
}

fn check_schema(source: &Source, doc: &Value, problems: &mut Vec<String>) {
    let schema_file = tilde(&source.schema);
    let schema: serde_json::Value = match fs::read_to_string(&source.schema)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => {
            problems.push(format!("{schema_file}: could not be read as JSON: {e}"));
            return;
        }
    };
    let instance = match serde_json::to_value(doc) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("{}: {e}", tilde(&source.rules)));
            return;
        }
    };
    let validator = match jsonschema::validator_for(&schema) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("{schema_file}: not a usable schema: {e}"));
            return;
        }
    };
    for err in validator.iter_errors(&instance) {
        let at = err.instance_path.to_string();
        let at = if at.is_empty() { "/".to_string() } else { at };
        problems.push(format!("{} at {at}: {err}", tilde(&source.rules)));
    }
}

fn check_skill(source: &Source, name: &str, problems: &mut Vec<String>) {
    let file = source.skills.join(name).join("SKILL.md");
    let shown = tilde(&file);
    let Ok(text) = fs::read_to_string(&file) else {
        problems.push(format!(
            "{shown}: missing, so no agent can load skill `{name}`"
        ));
        return;
    };
    // A file written on Windows carries \r before every newline.
    let text = text.replace("\r\n", "\n");
    let front = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(f, _)| f));
    let Some(front) = front else {
        problems.push(format!(
            "{shown}: no frontmatter, so agents cannot list skill `{name}`"
        ));
        return;
    };
    let meta: Value = serde_yaml::from_str(front).unwrap_or(Value::Null);
    for key in ["name", "description"] {
        if meta
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            problems.push(format!("{shown}: frontmatter has no `{key}`"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmp::{Temp, source};

    /// A rules file in the shape the format block describes.
    fn rules(body: &str) -> String {
        format!(
            "title: MYRULES\nformat:\n  keys:\n    name: A label.\n    case: The situation.\n    rule: The directives.\nsections:\n{body}"
        )
    }

    const ONE_RULE: &str = "  - section: Comments\n    rules:\n      - name: One clause\n        case: ~\n        rule:\n          - Keep a comment to one clause.\n";

    #[test]
    fn a_canon_whose_rules_and_skills_are_sound_has_no_problems() {
        let t = Temp::new();
        t.write("rules.yaml", &rules(ONE_RULE));
        t.skill("skills/audit", "audit");
        t.write("house/HOUSE-RUST.md", "");
        let report = run(&source(t.path()));
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.rules, Some(1));
        assert_eq!(report.skills, 1);
        assert_eq!(report.house, 1);
    }

    #[test]
    fn every_rule_is_counted_wherever_it_sits_in_the_file() {
        let t = Temp::new();
        let extra = "  - section: Tests\n    rules:\n      - name: What earns a test\n        case: ~\n        rule:\n          - Write a test only for a regression.\n      - name: Test names\n        case: ~\n        rule:\n          - Name a test after the invariant it pins.\n";
        t.write("rules.yaml", &rules(&format!("{ONE_RULE}{extra}")));
        assert_eq!(run(&source(t.path())).rules, Some(3));
    }

    #[test]
    fn a_rule_with_its_keys_out_of_the_order_format_lists_is_a_problem() {
        let t = Temp::new();
        let swapped = "  - section: Comments\n    rules:\n      - name: One clause\n        rule:\n          - Keep a comment to one clause.\n        case: ~\n";
        t.write("rules.yaml", &rules(swapped));
        let problems = run(&source(t.path())).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("One clause"), "{}", problems[0]);
    }

    #[test]
    fn a_rule_using_a_key_the_format_block_does_not_define_is_a_problem() {
        let t = Temp::new();
        let extra_key = "  - section: Comments\n    rules:\n      - name: One clause\n        case: ~\n        rule:\n          - Keep a comment to one clause.\n        checks: []\n";
        t.write("rules.yaml", &rules(extra_key));
        let problems = run(&source(t.path())).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("checks"), "{}", problems[0]);
    }

    #[test]
    fn a_rules_file_that_breaks_its_schema_is_a_problem() {
        let t = Temp::new();
        t.write("rules.yaml", &rules(ONE_RULE));
        t.write(
            "rules.schema.json",
            "{\"type\": \"object\", \"required\": [\"intro\"]}",
        );
        let problems = run(&source(t.path())).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn a_rules_file_that_is_not_yaml_only_has_to_be_there() {
        let t = Temp::new();
        let mut s = source(t.path());
        s.rules = t.write("MYRULES.md", "# My rules\n\nWhatever I like, in prose.\n");
        let report = run(&s);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(
            report.rules, None,
            "a file that is not YAML has no rules to count"
        );
    }

    #[test]
    fn a_rules_file_that_is_missing_is_a_problem() {
        let t = Temp::new();
        let problems = run(&source(t.path())).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("rules.yaml"), "{}", problems[0]);
    }

    #[test]
    fn a_canon_with_no_rules_file_at_all_is_valid() {
        let t = Temp::new();
        let mut s = source(t.path());
        s.rules = std::path::PathBuf::new();
        let report = run(&s);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.rules, None);
    }

    #[test]
    fn a_skill_with_no_skill_md_is_a_problem_naming_it() {
        let t = Temp::new();
        t.write("rules.yaml", &rules(ONE_RULE));
        t.dir("skills/half-done");
        let problems = run(&source(t.path())).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("half-done/SKILL.md"),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_skill_whose_frontmatter_has_no_name_or_description_is_a_problem() {
        let t = Temp::new();
        t.write("rules.yaml", &rules(ONE_RULE));
        t.write("skills/audit/SKILL.md", "---\nname: audit\n---\n\nbody\n");
        let problems = run(&source(t.path())).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("description"), "{}", problems[0]);
    }

    #[test]
    fn a_skill_md_written_on_windows_still_reads() {
        let t = Temp::new();
        t.write("rules.yaml", &rules(ONE_RULE));
        t.write(
            "skills/audit/SKILL.md",
            "---\r\nname: audit\r\ndescription: audits\r\n---\r\n\r\nbody\r\n",
        );
        let problems = run(&source(t.path())).problems;
        assert!(problems.is_empty(), "{problems:?}");
    }
}
