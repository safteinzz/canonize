# canonize (`canon`)

> **Canonical:** [gitlab.com/safteinzz/canonize](https://gitlab.com/safteinzz/canonize) · **Mirror:** [github.com/safteinzz/canonize](https://github.com/safteinzz/canonize)

<!-- desc:start -->
bring your rules to any coding agent - one canon of rules, conventions and skills for the agents you use
<!-- desc:end -->

## Install

```bash
cargo install canonize
canon self check   # is a newer release out?
canon self update  # install the latest
```

No cargo yet? Rust installs the same way on every distro: [rustup.rs](https://rustup.rs).

## Set up around what you already have

![the setup wizard finding a moved canon, then F fixing every agent's setup](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/setup.gif)

```bash
canon            # first run: six questions, each already answered with what it found
canon setup      # the same answers, taken as it found them, with nothing to press
canon setup -n   # printed, with nothing written
```

Every answer is also a flag, so a script or an agent can set it up without the wizard: `canon setup --root ~/dotfiles/canon --rules MYRULES.yaml --schema "" --conventions 'CONVENTIONS-*.md' --skills skills --projects ~/dev --projects ~/work`, where `""` turns one off and `--projects` repeats.

The first run asks which folder holds your rules, which file in it is your rules, which schema checks them, which files are your conventions, where your skills should live and where your projects are. Each answer comes pre-filled with what canonize found, and a note says why (your `~/.claude/CLAUDE.md` already imports a rules file from that folder, say), so you can keep it or change it. A last box says in plain sentences what it will write, and nothing is written before you say yes: a `canonize.toml` in your folder and a `~/.config/canonize` shortcut to it.

## See what every agent has

![the agents tab: a card opened with Enter, a broken rules import fixed with f, the skills line linking them all](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/agents.gif)

```bash
canon            # the dashboard: Agents, Skills, Conventions and MCPs tabs
canon status     # the same, printed
```

![canon status printing the agents and projects tables with what is broken](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/status.png)

The **Agents** tab lists your agents, a green dot when one is fully wired, yellow when something needs fixing, hollow when it is not installed, and shows the selected one's card: its rules file, whether it reads a project's CANON.md, and a summary of its skills. `j`/`k` picks the agent and Enter opens its card, where `j`/`k` walks the lines, `f` or `d` acts on one and Esc goes back to the list.

The **Skills** tab has a row per skill and a column per agent. Each cell says `linked` (wired to your canon), `unwired` (`f` wires it), `broken` (wired to the wrong thing, such as a folder that moved), `own` (a skill an agent keeps itself that your canon does not have yet), `foreign` (a real file, left alone) or `-` (the agent is not installed).

`canon status` exits 1 when anything is missing or broken, so a login script or a CI job can gate on it. A `foreign` or `own` cell is not drift, because it may be exactly what you want; `canon status` ends by naming each one and what you can do about it, and `canon status --strict` exits 1 on those too.

## Fix what drifted, and take it back

```bash
canon fix            # link what is missing, repoint what is broken
canon fix -n         # print what would change and change nothing
canon delete -a pi   # delete everything canonize made for one agent
```

canonize only ever touches what it made: symlinks that point into your source, `@path` lines naming a file in it, a project's `CANON.md` and the lines it adds to `CLAUDE.local.md` and `.gitignore`, and your MCP servers' entries in each agent's config, where a file it empties is deleted only when nothing else is left in it. A real file, or a link pointing at something that is really there, is reported and left alone, and nothing is ever copied back into your source. A link with nothing behind it is deleted wherever it pointed, because the agent lists that skill and finds nothing. Agents sharing a folder get one link.

How the rules reach an agent depends on what the agent understands. Claude resolves `@path` imports, so canonize adds one line to `~/.claude/CLAUDE.md` and leaves the rest of that file yours. An agent without imports gets its instructions file as a symlink to your rules file.

## Bring an agent's own skills in

![the skills tab: a skill Claude kept itself adopted with Enter, every skill linked to Claude with a, then every missing link made with F](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/skills.gif)

```bash
canon adopt claude my-skill   # move it into your source, leave a link in its place
canon adopt --all             # every skill every agent keeps itself
canon adopt --all -n          # print what that would move, move nothing
```

Enter works like in the conventions tab: it links a missing skill, unlinks a linked one, or adopts an `own` one; `a` and `d` open a choice of this cell, every skill for this agent, or this skill for every agent. A skill an agent keeps itself shows as `own` in that agent's column. Adopting it moves the whole folder, SKILL.md and scripts, into your canon and leaves a link in its place, so the agent sees no difference. `F` never adopts: moving your files is always one at a time. `d` on an `own` skill also offers to delete the folder itself, and `d` on a skill of your own canon offers to delete that, every agent's link to it included; both sit behind a red box that only unlocks once you type the skill's name, because nothing else has a copy. Folders without a SKILL.md, like the one Claude fills with the skills it syncs, are not skills and get no row, wherever they sit: one inside your canon is named by `canon validate` for what it is, with the skills it holds (`skills/synced: not a skill but a folder holding 2 (docx, pptx)`).

## Choose who reads which convention

![the projects tab: a broken import repointed with Enter, then one convention added to every project with a](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/projects.gif)

```bash
canon            # tab to Conventions: a column per convention, a row per agent and per project
canon status     # prints the same table under the agents one
```

A convention either fits a project (how your Rust crates ship) or fits an agent wherever it runs (that it is on WSL, how to reach your GitLab). The **Conventions** tab has both: the rows under `every project` are your agents, and one imported there is read by that agent in every project; the rows under `projects` are the projects canonize found. Enter works like a checkbox: it adds or repoints an import, or removes one that is there. `a` and `d` open a choice of how far to go: this cell, every convention in this row, or this convention in every agent or every project, each with how many changes it makes; `o` opens the file the import sits in. When your canon moves, every import of a convention that is gone shows as broken and `F` (or `canon fix`) repoints them all at once. `canon fix` never adds one: who reads which convention is only ever what you chose.

Tell canonize where your projects live (`projects = ["~/dev"]`, which setup asks for) and it finds every project with a `CLAUDE.md` or `AGENTS.md` up to three levels down. An import naming a file that exists nowhere is listed under the project.

### Where the imports live

An agent's own imports go where that agent reads them, and a convention you already import in `~/.claude/CLAUDE.md` by hand shows as `imported`:

```
Claude     an `@` line in ~/.claude/CLAUDE.md, beside your rules
pi         an `@` line in ~/.pi/agent/CANON.md, which canonize's extension loads
opencode   the file's path in "instructions" in ~/.config/opencode/opencode.json
Codex      cannot load another file, so it gets no conventions
```

Each project keeps its convention imports in its own `CANON.md`, gitignored and written only by canonize, so your personal paths never land in a tracked file and your own `CLAUDE.md` and `AGENTS.md` lines are never touched. Every agent loads it its own way:

```
Claude     `@CANON.md` in the project's CLAUDE.local.md, which is gitignored too
pi         the same extension, written to ~/.pi/agent/extensions/canonize.ts
opencode   "instructions": ["CANON.md"] in ~/.config/opencode/opencode.json
Codex      cannot load another file, so it gets no conventions
```

The `reads CANON.md` row of the agents tab shows which agents are wired. Convention imports still sitting in a project's CLAUDE.md or AGENTS.md show as broken, and a fix moves them into CANON.md, adds `CANON.md` and `CLAUDE.local.md` to the project's `.gitignore`, and writes `@CANON.md` into its CLAUDE.local.md. Claude reads AGENTS.md on its own only while a project has neither CLAUDE.md nor CLAUDE.local.md, so in a project with just AGENTS.md the CLAUDE.local.md imports it as well. A `@CANON.md` line in CLAUDE.md, where canonize 0.2.0 put it, moves to CLAUDE.local.md on the next fix, because CLAUDE.md is often tracked.

Your conventions live outside the project, and Claude loads files from outside a project only after you allow it there: the first time you open `claude` in a project, say yes to its question about external imports. Until you do, `canon status` lists the project under `needs you`. A CLAUDE.local.md you track in git, or keep as a link, is left alone, and `canon status` says so.

## Give your agents the same MCP servers

```bash
canon            # tab to MCPs: a column per server, a row per agent and per project
canon status     # prints the same table under the conventions one
```

Describe each server once, with `n` in the tab (`e` edits one, `D` deletes it from your canon with its token) or by hand in your canon's `mcp.toml`, and canonize writes it into every agent the way that agent wants it:

```toml
[esb]
url = "https://esb.example.com/mcp"
token = true                   # sent as `Authorization: Bearer`, the token typed in the tab

[docs]
url = "https://docs.example.com/mcp"
bearer_env = "DOCS_TOKEN"      # or read from $DOCS_TOKEN, if you keep it in your shell

[files]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
env = { DEBUG = "1", API_KEY = "${API_KEY}" }
```

No token is ever in your canon, which is in your dotfiles. Type one in the server's form and canonize keeps it in a file of its own outside your canon (`~/.local/share/canonize/tokens/`, readable by you alone), which every agent reads each time it connects: Claude through a `headersHelper`, opencode through `{file:…}`, pi through `!command`. A new token is `e` on the server, typed, and Enter: Claude uses it on its next connection, the others in their next session, and nothing has to be rewritten. Codex reads no file, so it reads that server's token from a variable named after it (`ESB_TOKEN` for `esb`), which `canon status` names. Any other value is plain text or one whole `${NAME}`, which every agent reads from your environment, and `canon status` names a variable a server needs that is not set. A server added on an agent's row goes into that agent's own config and so into every project: Claude through `claude mcp add-json -s user`, codex's `~/.codex/config.toml`, pi's `~/.pi/agent/mcp.json`, opencode's `opencode.json`. One added on a project's row goes to every agent, there only: Claude's local scope for that project (Claude keeps one list per git repo, so a project below its repo's root gets it for every agent but Claude), and `.codex/config.toml`, `.pi/mcp.json` and `opencode.json` in the project, each added to its `.gitignore` and never written when git already tracks it. Codex and pi read a project's file only in a project you have told them to trust. When you change a server in `mcp.toml`, every copy shows as broken and `F` (or `canon fix`) rewrites them; keys you added to an entry yourself, such as a timeout, are kept.

## Move your canon

![canon move renaming the canon folder and repointing every import, link and shortcut that named the old path](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/move.png)

```bash
canon move canon ~/dotfiles/canon        # the whole folder
canon move house conventions             # something inside it, renamed
canon move WSL.md conventions            # or into a folder that is there
canon move MYRULES.md ~/                 # something out of it
canon move ~/old/canon ~/dotfiles/canon  # one you already moved by hand: repoint only
```

It moves first, then repoints everything that named the old path: the imports in every agent's file, conventions you import globally, every project's `CANON.md`, every link into your canon, the `~/.config/canonize` shortcut and any absolute path in `canonize.toml`. The move runs alone and stops the command if it fails, so nothing is ever repointed at a folder that is not there, and because repointing is worked out from what the files say now, running the same command again finishes a move that stopped halfway.

## Keep your canon valid

```bash
canon validate   # the rules file against its schema, and every skill's SKILL.md
```

`canon validate` holds a YAML rules file to its schema. The rules file is YAML with a `format` block that says what each key of a rule means and in which order they go. `canon validate` validates it against `rules.schema.json`, flags a rule whose keys are out of that order or use a key `format.keys` does not define, and flags a skill with no `SKILL.md` or no `name` and `description` in its frontmatter.

## Start a source

```bash
canon init                    # lay it out in your config folder
canon init ~/dotfiles/agents  # or anywhere, then point CANONIZE_SOURCE at it
```

`init` never overwrites a file, so running it on a folder you already have only adds what is missing.

## Commands

```bash
canon status [-a NAME] [--strict] [--json]   # the table, exit 1 on drift
canon fix [-a NAME] [-n]                     # fix what is missing or broken
canon delete [-a NAME] [-n]                  # delete what canonize made
canon validate [--json]                      # validate the rules file, the skills and mcp.toml
canon setup [-n] [--root DIR] [--rules NAME] # set up, wizard or flags
canon adopt [AGENT] [SKILL] [--all] [-n]     # move an agent's own skill into the source
canon move <WHAT> <TO> [-n]                  # move your canon, or something in it
canon evict [NAME] [AGENT] [--all] [-n]      # move what is not a skill back to an agent
canon config set <KEY> <VALUE>... [-n]       # change a setting, comments and all
canon init [DIR] [-n]                        # lay out a new source
```

`-a` limits a command to one agent; `canon <command> --help` has the details.

## Keys

| key | does |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `Tab` | next tab |
| `F` | fix everything this tab shows |
| `v` | validate your canon |
| `E` | edit canonize.toml |
| `r` | read everything from disk again |
| `?` | help |
| `q` `Esc` `Ctrl-c` | quit |

Each tab's own keys are on its bottom bar, and `?` lists them all.

## Driving it from a script

Nothing in the CLI ever prompts, so an agent can do the whole job: `canon setup`, then `canon status --json` to see what is wrong, `canon fix` to wire it, `canon adopt --all` to bring in what the agents kept to themselves, and `canon status --strict` to be sure nothing is left. The one step only a person can take is Claude's own question about external imports, asked the first time `claude` opens in a project, and `--strict` counts a project until it is answered. `--json` on `status` and `validate` prints absolute paths and these state names, which do not change with the wording of the tables: `linked`, `unwired`, `broken`, `foreign`, `own`, `na`, `off`, `absent`, and under `conventions` a cell is `imported` or `none` where the tables say `-`, and under `mcp` it is `added` or `none`. A project's `claude` is `approved`, `not_asked` or `declined`, whether Claude may load its conventions. Each entry under `unsettled` carries the advice that `canon status` prints for it.

## Where it keeps things

Your canon is `$CANONIZE_SOURCE`, else your config folder: `~/.config/canonize` on Linux, `~/Library/Application Support/canonize` on macOS (a symlink to a folder in your dotfiles works). It holds:

```
canonize.toml        which agents, and how each is wired (all optional)
rules.yaml           the rules every agent follows (any format; `""` for none)
rules.schema.json    the shape `canon validate` holds them to
conventions/*.md     conventions, imported by the projects or agents they fit
mcp.toml             your MCP servers, a table each
skills/<name>/       one folder per skill, with its SKILL.md
```

Every file name is a default `canonize.toml` can change, and `""` turns one off: no schema, no conventions, or no rules file at all when you keep your rules in each agent's own file. Each agent canonize knows (claude, codex, pi, opencode) has built-in paths; set only what differs, or add an agent it does not know with its `home`, `rules` and `skills`.

An agent whose `skills_mode` is `folder` reads your canon's skills folder as its own, so everything it writes there lands in your canon, and in your dotfiles with it. `canon status` says so as soon as something that is not a skill turns up in there, and `skills_mode = "per-skill"` links one skill at a time instead, keeping what the agent writes on its own side. Changing that setting is something `canon fix` finishes: the folder link it made becomes a folder of one link per skill. The whole repair is three commands and no shell:

```bash
canon config set agents.claude.skills_mode per-skill   # one setting, every comment kept
canon fix -a claude                                    # the folder link becomes one link per skill
canon evict synced claude                              # the bucket goes back to the agent that wrote it
```

`canon config set` takes `projects`, `source.<name>` or `agents.<agent>.<name>`, writes the value on that key's own line, and puts the file back untouched when the result would not load. `canon evict` only moves what is not a skill, and refuses while the agent's skills folder is a link into your canon, since that would move nothing.

## Compatibility

Linux and macOS. Windows is missing because the wiring is symlinks.

## License

AGPL-3.0-only
