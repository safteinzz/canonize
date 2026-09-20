# canonize (`canon`)

> **Canonical:** [gitlab.com/safteinzz/canonize](https://gitlab.com/safteinzz/canonize) · **Mirror:** [github.com/safteinzz/canonize](https://github.com/safteinzz/canonize)

<!-- desc:start -->
bring your rules to any coding agent - one canon of rules, house files and skills for the agents you use
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
canon setup -n   # the same answers, printed, with nothing written
```

The first run asks which folder holds your rules, which file in it is your rules, which schema checks them, which files are your house files, where your skills should live and where your projects are. Each answer comes pre-filled with what canonize found, and a note says why (your `~/.claude/CLAUDE.md` already imports a rules file from that folder, say), so you can keep it or change it. A last box says in plain sentences what it will write, and nothing is written before you say yes: a `canonize.toml` in your folder and a `~/.config/canonize` shortcut to it.

## See what every agent has

![the agents tab: a card opened with Enter, a broken rules import fixed with f, the skills line linking them all](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/agents.gif)

```bash
canon            # the dashboard: Agents, Skills and Projects tabs
canon status     # the same, printed
```

![canon status printing the agents and projects tables with what is broken](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/status.png)

The **Agents** tab lists your agents, a green dot when one is fully wired, yellow when something needs fixing, hollow when it is not installed, and shows the selected one's card: its rules file, whether it reads a project's CANON.md, and a summary of its skills. `j`/`k` picks the agent and Enter opens its card, where `j`/`k` walks the lines, `f` or `d` acts on one and Esc goes back to the list.

The **Skills** tab has a row per skill and a column per agent. Each cell says `linked` (wired to your canon), `missing` (`f` wires it), `broken` (wired to the wrong thing, such as a folder that moved), `own` (a skill an agent keeps itself that your canon does not have yet), `foreign` (a real file, left alone) or `-` (the agent is not installed).

`canon status` exits 1 when anything is missing or broken, so a login script or a CI job can gate on it.

## Fix what drifted, and take it back

```bash
canon fix            # link what is missing, repoint what is broken
canon fix -n         # print what would change and change nothing
canon delete -a pi   # delete everything canonize made for one agent
```

canonize only ever touches two kinds of thing: symlinks that point into your source, and `@path` lines naming a file in it. A real file, or a link pointing anywhere else, is reported and left alone, and nothing is ever copied back into your source. Agents sharing a folder get one link.

How the rules reach an agent depends on what the agent understands. Claude resolves `@path` imports, so canonize adds one line to `~/.claude/CLAUDE.md` and leaves the rest of that file yours. An agent without imports gets its instructions file as a symlink to your rules file.

## Bring an agent's own skills in

![the skills tab: a skill Claude kept itself adopted with Enter, every skill linked to Claude with a, then every missing link made with F](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/skills.gif)

```bash
canon adopt claude my-skill   # move it into your source, leave a link in its place
```

Enter works like in the projects tab: it links a missing skill, unlinks a linked one, or adopts an `own` one; `a` and `d` open a choice of this cell, every skill for this agent, or this skill for every agent. A skill an agent keeps itself shows as `own` in that agent's column. Adopting it moves the whole folder, SKILL.md and scripts, into your canon and leaves a link in its place, so the agent sees no difference. `F` never adopts: moving your files is always one at a time. `d` on an `own` skill also offers to delete the folder itself, behind a red box that only unlocks once you type the skill's name, because nothing else has a copy. Folders without a SKILL.md, like the one Claude fills with the skills it syncs, are not skills and get no row.

## See which project imports which house file

![the projects tab: a broken import repointed with Enter, then one house file added to every project with a](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/projects.gif)

```bash
canon            # tab switches to projects: a row per project, a column per house file
canon status     # prints the projects table under the agents one
```

Tell canonize where your projects live (`projects = ["~/dev"]`, which setup asks for) and it finds every project with a `CLAUDE.md` or `AGENTS.md` up to three levels down. Each cell says whether that project imports that house file. Enter works like a checkbox: it adds or repoints an import, or removes one that is there. `a` and `d` open a choice of how far to go: this project, every house file in this project, or this house file in every project, each with how many changes it makes; `o` opens the project's `CLAUDE.md`. When your canon moves, every import of a house file that is gone shows as broken and `F` (or `canon fix`) repoints them all at once. An import naming a file that exists nowhere is listed under the project.

### Where a project's house imports live

Each project keeps its house imports in its own `CANON.md`, gitignored and written only by canonize, so your personal paths never land in a tracked file and your own `CLAUDE.md` and `AGENTS.md` lines are never touched. Every agent loads it its own way:

```
Claude     one `@CANON.md` line in the project's CLAUDE.md
pi         an extension canonize writes to ~/.pi/agent/extensions/canonize.ts
opencode   "instructions": ["CANON.md"] in ~/.config/opencode/opencode.json
Codex      cannot load another file, so it gets no house files
```

The `reads CANON.md` row of the agents tab shows which agents are wired. House imports still sitting in a project's CLAUDE.md or AGENTS.md show as broken, and a fix moves them into CANON.md, adds `CANON.md` to the project's `.gitignore` and `@CANON.md` to its CLAUDE.md.

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
canon status [-a NAME]          # the table, exit 1 on drift
canon fix [-a NAME] [-n]        # fix what is missing or broken
canon delete [-a NAME] [-n]     # delete what canonize made
canon validate                  # validate the rules file and the skills
canon setup [-n]                # set up around the rules your agents already read
canon adopt <AGENT> <SKILL>     # move an agent's own skill into the source
canon init [DIR]                # lay out a new source
```

`-a` limits a command to one agent; `canon <command> --help` has the details.

## Keys

| key | does |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `Tab` | next tab |
| `F` | fix everything this tab shows |
| `D` | delete everything this tab made |
| `v` | validate your canon |
| `e` | edit canonize.toml |
| `r` | read everything from disk again |
| `?` | help |
| `q` `Esc` `Ctrl-c` | quit |

Each tab's own keys are on its bottom bar, and `?` lists them all.

## Where it keeps things

Your canon is `$CANONIZE_SOURCE`, else your config folder: `~/.config/canonize` on Linux, `~/Library/Application Support/canonize` on macOS (a symlink to a folder in your dotfiles works). It holds:

```
canonize.toml        which agents, and how each is wired (all optional)
rules.yaml           the rules every agent follows (any format; `""` for none)
rules.schema.json    the shape `canon validate` holds them to
house/*.md           house rules, imported by the projects they fit
skills/<name>/       one folder per skill, with its SKILL.md
```

Every file name is a default `canonize.toml` can change, and `""` turns one off: no schema, no house files, or no rules file at all when you keep your rules in each agent's own file. Each agent canonize knows (claude, codex, pi, opencode) has built-in paths; set only what differs, or add an agent it does not know with its `home`, `rules` and `skills`.

## Compatibility

Linux and macOS. Windows is missing because the wiring is symlinks.

## License

AGPL-3.0-only
