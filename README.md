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
canon setup      # the same answers with nothing to press, and every one is also a flag
canon setup -n   # printed, with nothing written
```

It starts from what your agents already read, such as the rules file an agent already imports, and nothing is written until you say yes.

## See what every agent has

![the agents tab: a card opened with Enter, a broken rules import fixed with f, the skills line linking them all](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/agents.gif)

Legend: green dot fully wired · yellow dot something needs fixing · hollow dot not installed

```bash
canon            # the dashboard: Agents, Skills, Conventions and MCPs tabs
canon status     # the same, printed
```

![canon status printing the agents and projects tables with what is broken](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/status.png)

`canon status` exits 1 when something drifted, so a login script or a CI job can gate on it. A file that is yours is never counted as drift: it is named at the end, with what you can do about it.

## Fix what drifted, and take it back

```bash
canon fix                 # link what is missing, repoint what is broken
canon fix -n              # print what would change and change nothing
canon delete -a <agent>   # delete everything canonize made for one agent
```

canonize only ever touches what it made: links into your canon, the lines it wrote, its own gitignored files and your MCP servers' entries in each agent's config. A real file is reported and left alone, and nothing is ever copied into your canon.

## Bring an agent's own skills in

![the skills tab: a skill an agent kept itself adopted with Enter, every skill linked to that agent with a, then every missing link made with F](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/skills.gif)

```bash
canon adopt <agent> my-skill  # move it into your canon, leave a link in its place
canon adopt --all             # every skill every agent keeps itself
canon adopt --all -n          # print what that would move, move nothing
```

Adopting moves the whole folder into your canon and leaves a link behind, so the agent sees no difference. `F` never adopts, because your files only ever move one at a time.

## Give each project the conventions it needs

Rust rules in your crates, web rules in your sites, and every agent reading the same ones.

![the projects tab: a broken import repointed with Enter, then one convention added to every project with a](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/projects.gif)

```bash
canon                           # tab to Conventions: who reads which convention
canon add rust --every project  # every project reads rust
canon add tui-vi .              # this project reads tui-vi
canon remove tui-vi .           # and stops
canon add wsl -a <agent>        # that agent reads wsl in every project
```

Imports live in each project's gitignored `CANON.md`, so your paths never land in the repo. An agent that asks before loading files from outside a project asks once, and `canon status` lists the project until you say yes.

## Give every agent the same MCP servers

```bash
canon            # tab to MCPs: c describes a server, Enter adds it to an agent or a project
canon fix        # rewrite every copy after you change one
```

Describe each server once, in the tab or in your canon's `mcp.toml`:

```toml
[esb]
url = "https://esb.example.com/mcp"
token = true                   # typed in the tab, kept outside your canon

[files]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
env = { API_KEY = "${API_KEY}" }
```

A token you type is kept outside your canon, readable by you alone, so your dotfiles never hold a secret.

## Move your canon

![canon move renaming the canon folder and repointing every import, link and shortcut that named the old path](https://gitlab.com/safteinzz/canonize/-/raw/main/readme-assets/move.png)

```bash
canon move canon ~/dotfiles/canon                     # the whole folder
canon move conventions/tui.md conventions/terminal.md # something inside it, renamed
canon move ~/old/canon ~/dotfiles/canon               # one you already moved by hand: repoint only
```

Every import, link and setting that named the old path is repointed, and running the same command again finishes a move that stopped halfway.

## Commands

```bash
canon validate [--json]               # check the rules file, every SKILL.md and mcp.toml
canon evict [NAME] [AGENT] [--all]    # move what is not a skill back to an agent
canon config set <KEY> <VALUE>...     # change a setting, comments kept
canon init [DIR]                      # lay out a new canon, never overwriting a file
```

Most commands take `-a NAME` for one agent and `-n` for a dry run, `canon <command> --help` has the details, and `?` in the dashboard lists every key.

Nothing prompts, so a script or an agent can do all of it: `--json` on `status` and `validate` prints for a machine, both exit 1 when something needs fixing, and `status --strict` also counts what needs you.

## Where it keeps things

```
<canon>/canonize.toml             which agents, and how each is wired (all optional)
<canon>/rules.yaml                the rules every agent follows
<canon>/rules.schema.json         the shape `canon validate` holds them to
<canon>/conventions/*.md          conventions, imported by the projects or agents they fit
<canon>/mcp.toml                  your MCP servers, a table each
<canon>/skills/<name>/            one folder per skill, with its SKILL.md
~/.local/share/canonize/tokens/   the tokens you type for an MCP server, outside your canon
```

`<canon>` is `$CANONIZE_SOURCE`, else `~/.config/canonize` (`~/Library/Application Support/canonize` on macOS), a link into your dotfiles works, and `canonize.toml` can rename any of these or turn one off with `""`.

## Compatibility

Linux and macOS. Windows is missing because the wiring is symlinks.

## License

AGPL-3.0-only
