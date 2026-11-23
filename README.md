# codex-cli

Personal tools and settings for the Codex CLI. Switch accounts with `codex-auth`, resume a local conversation from the native session picker, use the Solarized Osaka theme, and save implementation plans to Inkdrop.

## Install

The full installer requires the official Codex CLI and Fish on your `PATH`. The prebuilt session-sharing adapter runs on x86_64 Linux with kernel 6.1 or later.

From the project root, run:

```sh
./install.sh
```

The installer reuses an existing `codex-auth`. On a fresh machine, it installs version 0.4.0 under `~/.local` and adds `~/.local/bin` to the Fish search path if needed. It uses npm when Node.js and npm are available. On x86_64 Linux without them, it downloads and verifies the [standalone release](https://github.com/Loongphy/codex-auth/releases/tag/v0.4.0) using `curl`, `sha256sum`, and `tar`. The standalone binary requires glibc 2.28 or later. Both paths install to your home directory without `sudo`. The installer tests session sharing against your installed Codex CLI using synthetic conversations before installing the adapter.

To install AGENTS.md, the theme, and the skills using just the shell and standard Linux utilities, run:

```sh
./install.sh --settings-only
```

Both modes copy [AGENTS.md](AGENTS.md) to `$CODEX_HOME/AGENTS.md` (default `~/.codex/AGENTS.md`). If the destination already has different instructions, the installer saves a numbered backup beside it.

## Use

Open a new Fish terminal in your project directory. To add an account for the first time, run `codex-auth login`. To switch to a saved account and continue a conversation, run:

```fish
codex-auth switch
codex resume
```

The picker shows conversations from the current directory across accounts that share the same Codex home. Use `codex resume --all` to include other directories.

In Fish, use `codex auto-switch on` or `off` to control switching to a saved account when the current one runs out of quota. Use `codex auto-resume on` or `off` to control whether interrupted turns and Goals wait for quota to recover and resume in the same session. Both commands accept `status`. See the [session-sharing guide](session-sharing/README.md#continue-after-an-account-usage-limit).

## Solarized Osaka

In Codex, open `/theme`, select **solarized-osaka (custom)**, and press Enter. Codex saves your selection.

Designed for dark terminals, the theme uses gold for **bold text**, light blue for inline code and local file link targets, and underlined cyan for URLs. Paths written as ordinary text keep the paragraph color. The palette comes from the local Caelestia Solarized Osaka setup, with heading, link, and list colors inspired by Inkdrop Solarized Dark.

## CLI updates

The adapter lives in `~/.local/share/codex-session-sharing/`, the Fish function in `~/.config/fish/functions/`, and the theme in `$CODEX_HOME/themes/` (default `~/.codex/themes/`). Normal Codex CLI updates preserve these user files. If a new CLI release changes the app-server protocol, rerun the installer to check compatibility.

## Skills

Type `$` in Codex to browse installed skills. The installer copies the full folders from `skills/` into `~/.agents/skills/`, or updates an existing copy under `$CODEX_HOME/skills/` in place.

| Skill | What it does |
| --- | --- |
| `$ponytail` | Make the smallest maintainable change that meets the request. |
| `$inkdrop` | Draft an implementation plan and write it to a note you select. |
| [`$algo-tutor`](skills/algo-tutor/SKILL.md) | Practice a LeetCode or other algorithm problem with one Socratic question at a time. |

Start a tutoring session with the problem link and your current idea in one message:

```text
$algo-tutor https://leetcode.com/problems/longest-substring-without-repeating-characters/ I would start from every index and stop when I see a duplicate.
```

## Plan in Inkdrop

Set up [Inkdrop MCP for Codex](https://docs.inkdrop.app/reference/mcp-server#codex) first. In a regular Codex chat, enter `$inkdrop` to choose an existing note or a notebook for a new note. Search by title and select a result by number, or paste a note link or ID to go directly to it. You can copy the link from Inkdrop or drag the note into the terminal.

```text
$inkdrop inkdrop://note/<id>
```

The skill drafts the plan, saves it to the selected note, returns its link, and waits for approval before implementation. Codex reserves `/plan` for its built-in Plan Mode. The installer adds the skill under `~/.agents/skills/` so it stays available across official CLI updates.
