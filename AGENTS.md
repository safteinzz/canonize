<!--
AI-ONLY DOCUMENT. This file exists to give an AI agent the COMPLETE operating picture for this repo. Optimize for completeness and precision for the agent, not for human readability. Humans read README.md instead. FORMAT: machine-read; put each rule/point on ONE line, however long.
-->
# AGENTS.md
Working brief for an AI coding agent: the invariants, gotchas and decisions needed to change this project correctly.

## What it is
- canonize (binary `canon`) manages a SOURCE folder the user owns (rules file, house files, skills, `canonize.toml`) and wires it into each coding agent's own folders. It holds no rules itself; the crate ships only templates for `canon init`.
- Bare-first binary: bare `canon` opens the TUI; `status`, `fix`, `delete`, `validate`, `setup`, `adopt`, `init`, `self` are the plain commands over the same engines.

## Module map
- `src/config.rs`: finds the source (`$CANONIZE_SOURCE`, else `dirs::config_dir()/canonize`), parses `canonize.toml` (deny_unknown_fields), merges built-in agent defaults (claude, codex, pi) with overrides, path helpers `expand` (`~/`) and `tilde` (display and import lines), `house_files`, `skill_names`.
- `src/plan.rs`: `Plan::build` compares each agent with the source into `cells[row][agent]` (`State` + `Change` for fix + `Change` for remove), `foreign` (per agent, folders in a per-skill folder with no SKILL.md, not rows); agent-own skills (a real folder with a SKILL.md) become extra rows with `State::Own`, `Change::Adopt` for f and `Change::DeleteDir` for d; the delete only runs through `tui/typed.rs` (typed gate, the skill's name) and is filtered out of every bulk removal, `stale` (links into the source whose skill is gone). `Change::run` executes one change.
- `src/projects.rs`: walks `cfg.projects` up to depth 3 (skipping dot folders, `target`, `node_modules`, `dist`, `build`, never following links) for folders with `CLAUDE.md` or `AGENTS.md`. A project's house imports live in its gitignored `CANON.md` (`plan::CANON_FILE`). Per house file: imported (in CANON.md), broken (in CANON.md naming a gone file: ReplaceImport; or still in CLAUDE.md/AGENTS.md: MoveImport into CANON.md) or missing (AddImport into CANON.md, only on request). `wiring` per project: CANON.md in `.gitignore` (via `git check-ignore`, AppendLine) and `@CANON.md` in CLAUDE.md (AddImport); `fix` includes wiring for every project that imports a house file, and any add brings it along.
- `src/check.rs`: skips the rules checks when `rules` is empty or the file is not `.yaml`/`.yml` (the count is then left out of `counts`); otherwise validates the rules file (YAML parse, JSON schema via `jsonschema`, key order from the file's own `format.keys`, unknown keys) and each skill's `SKILL.md` frontmatter (`name`, `description`).
- `src/setup.rs`: first run. `detect_or_moved` (used by both the wizard and `canon setup`) falls back to `lost`, an import of a rules file that is gone whose name turns up in a folder next door. `detect` reads the default agents' instruction files for an `@path` import (or the file being a symlink) naming a rules file that exists and is not a house file; `Choice` holds the user's answers (`Pick::Existing|New|None`); `apply` writes starters for `New`, then `canonize.toml` with only non-default names (an empty value turns schema or house files off) and links `~/.config/canonize` to the source when neither it nor `$CANONIZE_SOURCE` is set; `adopt` renames a real skill folder into the source and symlinks it back (fails across filesystems by design).
- `src/init.rs`: writes the starter files into a folder, never overwriting; `init::config_text` fills `templates/canonize.toml` (every setting written out with a comment explaining it, TOML lists included) for both `init` and setup.
- `src/cli.rs`: plain commands and their exit codes. `src/selfcmd.rs`: `self update|check`, same as the other crates.
- `src/tui/`: `mod.rs` App, keys and loop (first run with no config proposes setup at once); `render.rs` frame; `alert.rs` `Note` (alert yellow, reader cyan); `confirm.rs` gate (remove) and offers (fix, adopt, set up) carrying an `Action` and the green shell commands (`Change::command`); `wizard.rs` the six setup questions (folder, rules, schema, house, skills, projects), pre-filled from `setup::detect` with a note saying why, ending in a summary offer that runs `setup::apply(Choice)`; `widgets.rs` house box furniture copied from easyssh.

## Invariants
- canonize only ever creates or removes (a) symlinks pointing into the source root and (b) `@path` lines naming a file in the source. It never deletes or overwrites a real file, never edits a link pointing outside the source, never copies anything into the source. `remove_link` refuses a non-symlink.
- The global rules import is also treated as a rename when an import of a gone file carries the rules file's name, so moving the canon folder is one `fix`.
- An import line is written as `@` + `tilde(path)`; detection compares trimmed lines exactly. An existing `@path` into the source whose file no longer exists is treated as a rename and replaced (`ReplaceImport`); one whose file exists (a house file imported globally) is the user's and left alone.
- Changes are deduplicated by target (`Change::key`), so agents sharing a folder (codex and pi on `~/.agents/skills`) get one link.
- Per-skill agents whose skills folder is itself a symlink are reported foreign, not converted.
- `status` and `validate` exit 1 on drift/problems; errors go to stderr with exit 1. Foreign entries alone do not count as drift.
- `config::load_from` canonicalizes the source root, so a source reached through `~/.config/canonize` is used by its real path; without this, imports and links would name the link and never match what agents already read.
- File writes go through the path (`fs::write`), so a symlinked instructions file stays a symlink.
- Unix only (`std::os::unix::fs::symlink`).

## CANON.md loaders
- Agents tab row `reads CANON.md` (only when `projects` is set): claude n/a (per project, via CLAUDE.md), pi `WriteFile` of `templates/pi-canonize.ts` to `~/.pi/agent/extensions/canonize.ts` (never overwritten), opencode `JsonInstruction` adding `CANON.md` to `instructions` in `opencode.json` (plain JSON only; a file with comments is refused with a message), anything else n/a.

## Defaults (best known, overridable)
- claude: home `~/.claude`, rules `~/.claude/CLAUDE.md` import, skills `~/.claude/skills` per-skill.
- codex: home `~/.codex`, rules `~/.codex/AGENTS.md` link, skills `~/.agents/skills` folder.
- pi: home `~/.pi/agent`, rules `~/.pi/agent/AGENTS.md` link, skills `~/.agents/skills` folder. Stock pi does not resolve `@` imports, hence link.
- opencode: home `~/.config/opencode`, rules `~/.config/opencode/AGENTS.md` link, skills `~/.config/opencode/skills` per-skill (from opencode.ai/docs/rules and /skills).
- An agent is acted on only when its `home` exists and it is enabled.

## Not built yet
- Guardrails (one allow/deny list translated into each agent's settings), MCP servers, hooks: planned as new rows fed by translation code.

## Keys
- Three tabs: Agents (list + card of `card_lines`, focus in `in_card`: Enter opens the card, Esc returns, f/d only act inside it; setup rows then a skills summary; `selected_row()` is `None` on the summary, where f/d act on all the agent's skill links), Skills (grid of `Plan::skill_rows` × agents, `srow`/`col`), Projects (grid, `prow`/`pcol`). `D` in Skills deletes every skill link for every agent.
- F and D act on the current tab only (agents: setup rows; skills: skill links and stale links; projects: imports and wiring); the CLI `canon fix` is the global one.
- Both tabs: `f` fix the selected cell (its `change`), `F` fix all in this tab, `d` delete the selected cell (its `undo`, red gate), `D` delete all (agents: every agent's undos; projects: every house import from every project; agents-only keys never fall through from the projects tab), Enter = `f` in agents and a toggle in projects; in projects `a`/`d` open `tui/scope.rs`, a picker of this cell / every house file in this project / this house file in every project, each carrying its changes, `v` validate, `e` edit canonize.toml, `r` reload, `tab` switch, `?` help, `q` quit. Agents: `f` on an `own` cell adopts (`Change::Adopt`), and `F` never includes adopts. Projects: `o` opens the host file. No per-agent bulk keys: the CLI's `-a` covers that.

## Demo rig
- `demo/stage.sh` builds `demo/home`: a canon just moved from `~/dotfiles/development` to `~/dotfiles/canon` (starter rules and schema, `house/HOUSE-RUST.md`, `house/HOUSE-TUI.md`, skills `code-review` and `release`), a Claude whose `CLAUDE.md` still imports the old path plus its own `meeting-notes` and `synced` skills, installed Codex, pi and opencode with nothing wired, and projects `dev/crates/api`, `dev/crates/cli` (house imports from the old folder) and `dev/web/site`. `up` leaves it unset (wizard opens); `ready` also writes `canonize.toml` and the `~/.config/canonize` link, drift left in place.
- Tapes, one per picture: `setup.tape` (setup.gif: six Enters, y, F, y), `agents.tape` (agents.gif: open a card, fix a line, link the skills, Esc, F), `skills.tape` (skills.gif: adopt with f, F), `projects.tape` (projects.gif: Enter repoints, F, a… every project, d…), `shots.tape` (status.png). Run from `demo/`, then `./stage.sh down`.

## Audit fixes (keep these true)
- `import_cell` refuses a file that is a symlink into the canon, and `AddImport` refuses a symlink target: writing an import through the link would edit the rules file itself.
- `edit_lines` drops only the lines it is told to; blank lines are the user's.
- A `RemoveImport` that empties a `CANON.md` deletes the file.
- Every house import of the same file is moved, `CLAUDE.md` and `AGENTS.md` copies alike (`Change::Batch`).
- `canon status -a NAME` exits on the drift it printed, not on other agents'.
- `canon delete` without `-a` also takes back the loaders (pi extension, opencode setting) and every project's CANON.md, `@CANON.md` line and `.gitignore` line.
- `demo/stage.sh run|shell` refuse without a stage; `ready` sets up by running `canon setup`, so the staged config is the one the tool writes.
