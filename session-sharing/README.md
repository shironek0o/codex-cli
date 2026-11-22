# Local session sharing for Codex

Version 1.4.0 lets you switch accounts and continue conversations in the native Codex interface. The `codex resume` picker, `codex resume --last`, and the in-app `/resume` command can find sessions from other accounts that use the same local `CODEX_HOME`.

## Install

For the account switcher, adapter, and Solarized Osaka theme together, run `./install.sh` from the project root. To install just the adapter, install the official Codex CLI and Fish, then run this command from the directory containing `install.fish`:

```fish
fish install.fish
```

The prebuilt binary runs on x86_64 Linux with kernel 6.1 or later. Before changing the Fish integration, the installer tests two synthetic accounts in a temporary directory. It checks that the picker filters by workspace and that two connections can resume the same conversation. A failed check stops installation.

## Switch accounts and resume

Open a new Fish terminal in your project directory and run:

```fish
codex-auth switch
codex resume
```

You can also use another account switcher. By default, the picker lists conversations from the current working directory. Use `-C` or `--cd` to specify a directory, or use `codex resume --all` (or **All** in the picker) to browse other directories.

When upgrading from 1.0.0, exit any Codex windows started with that adapter before opening a new terminal. If another independent app-server holds a conversation open, close the conversation in that app and retry. Leave its lock files in place so other running sessions can finish safely.

## Continue after an account usage limit

Save each ChatGPT account with `codex-auth login`, then use the installed `codex` Fish function. When a turn submitted through that terminal fails with an account usage limit, the adapter checks the saved accounts for available quota. It switches to one with confirmed quota if the shared daemon has no other running turns. If the daemon restarts to load the new login, the native interface reconnects to the same session. The adapter then asks Codex to continue the unfinished work using that session's history.

Goal mode also triggers a handoff when a turn started by the goal hits the account usage limit. The adapter reactivates the existing goal through Codex in the same session. A paused goal or a goal that exhausted its own token budget retains its state.

When an immediate handoff is unavailable and the terminal reports that the goal is queued, a background process checks the active account's quota. It reactivates the same goal once Codex confirms that included usage is available. The process keeps checking after you close the terminal and can recover the same goal from another quota stop. Pausing or clearing the goal cancels the wait. Keep the machine running and the shared Codex daemon available. This uses `setsid` from util-linux.

Automatic continuation attempts one account switch per submitted turn or goal run. It requires `codex-auth` 0.4.0 or later and a Codex CLI with native reconnect support, verified with 0.171.0. The normal `codex exec` and explicit `--remote` paths keep their existing behavior.

In Fish, check or change the setting with these commands:

```fish
codex auto-switch status
codex auto-switch off
codex auto-switch on
```

The setting defaults to on and is saved per `CODEX_HOME` (default `~/.codex`). Turning it off disables automatic quota checks, account switching, and Goal recovery for subsequent Codex launches. Existing sessions and queued Goals keep running with their original settings. Use `CODEX_AUTO_SWITCH_OFF=1 codex` to turn it off for one launch.

## Install on another machine

Copy the `codex-session-sharing` executable, the three Fish files, and this README into one directory on the destination machine. Run `fish install.fish` there. The statically linked executable runs without Rust. The bundle contains the tool only. Credentials, conversations, and machine settings stay on their original machine.

For a different CPU architecture or an older kernel, build the adapter from the included Rust source with `cargo build --locked --release`. Copy `target/release/codex-session-sharing` alongside `install.fish` before installing.

## How it works

The Fish function looks up the official `codex` executable on `PATH` each time it runs. For interactive sessions, the adapter uses the CLI's `--remote` interface and a private local Unix socket to forward requests to the official Codex daemon. For `thread/list` and `thread/search`, it sets `currentAccountOnly` to `false`. It passes the working directory explicitly because the remote picker defaults to **All** when the directory is missing. Archive filters, search terms, pagination, and explicit `--all` selections are preserved.

All windows use the same daemon and session writer. Codex and the account switcher continue to manage the daemon and its credentials. Noninteractive commands, login, MCP management, explicit remote targets, named profiles, and `--no-daemon` go directly to the official CLI, bypassing the shared picker.

Update Codex through your usual installer. The Fish function and adapter live separately in `~/.config/fish/functions/codex.fish` and `~/.local/share/codex-session-sharing/`. If a future Codex release changes the app-server protocol or removes a required option, the adapter may need an update.

## Disconnect and reconnect

When Codex prints `Disconnected from this task. Any running work continues.`, the frontend has closed and the daemon can keep working. To stop a running turn, open `codex agents`, select the task, and use the stop shortcut shown on screen.

Version 1.2.0 displays a single reconnect command, `codex resume <session-id>`, with the full session ID. You can also run `codex resume` to open the picker. The adapter removes its temporary `--remote unix:///tmp/.../rpc.sock` argument from the exit message. A pseudoterminal preserves colors, control sequences, and window size. If you paste an older reconnect command containing an expired adapter socket, the adapter opens a fresh local connection. Explicit remote targets still go to the address you specify.

## Disable the integration

To use the official CLI directly for one invocation, run:

```fish
command codex resume
```

To remove the Fish integration and clear the loaded function in the current shell, run:

```fish
fish ~/.local/share/codex-session-sharing/uninstall.fish
functions -e codex
```

Session data and the official executable stay in place. When migrating from the earlier custom 0.169.0 build, the installer tests the official executable, restores its original entry point, and retains the custom build as a backup.
