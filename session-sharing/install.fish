#!/usr/bin/fish --no-config

set -l bundle (path resolve -- (path dirname -- (status filename)))
set -l target "$HOME/.local/share/codex-session-sharing"
set -l upstream (command -s codex)
if test -z "$upstream"
    printf 'Error: install the official Codex CLI first.\n' >&2
    exit 1
end
if not test -x "$bundle/codex-session-sharing"
    printf 'Error: the portable adapter executable is missing.\n' >&2
    exit 1
end

# Test the official CLI with synthetic sessions before installing the adapter.
set -l previous_entry "$HOME/.local/opt/codex-native-resume/0.169.0/original-entry"
set -l custom_entry "$HOME/.local/opt/codex-native-resume/0.169.0/package/bin/codex"
set -l replace_entry 0
if test -L "$HOME/.local/bin/codex"; and test (readlink -f "$HOME/.local/bin/codex") = "$custom_entry"; and test -x "$previous_entry"
    set upstream (readlink -f "$previous_entry")
    set replace_entry 1
end
printf '  Checking session sharing with your Codex CLI...\n'
"$bundle/codex-session-sharing" --self-test --binary "$upstream" >/dev/null
or begin
    printf 'Error: installation stopped before changing your Codex command.\n' >&2
    exit 1
end

mkdir -p "$target" "$HOME/.config/fish/functions"
or exit 1
set -l backup "$HOME/.local/state/codex-session-sharing/"(date +%Y%m%d-%H%M%S)
mkdir -p "$backup"
or exit 1
if test -f "$HOME/.config/fish/functions/codex.fish"
    cp "$HOME/.config/fish/functions/codex.fish" "$backup/codex.fish"
    or exit 1
end
if test "$bundle" != "$target"
    # Replace the executable by rename so running clients keep their current inode.
    set -l staged (mktemp "$target/.adapter.XXXXXXXX")
    or exit 1
    if not install -m755 "$bundle/codex-session-sharing" "$staged"; or not mv -fT "$staged" "$target/codex-session-sharing"
        rm -f "$staged"
        exit 1
    end
    for name in codex.fish install.fish uninstall.fish README.md
        cp "$bundle/$name" "$target/$name"
        or exit 1
    end
end
if test "$replace_entry" -eq 1
    cp -P "$HOME/.local/bin/codex" "$backup/codex-entry"
    or exit 1
    cp -P "$previous_entry" "$HOME/.local/bin/codex.session-sharing-new"
    or exit 1
    mv -Tf "$HOME/.local/bin/codex.session-sharing-new" "$HOME/.local/bin/codex"
    or exit 1
end
cp "$target/codex.fish" "$HOME/.config/fish/functions/codex.fish"
or exit 1
printf '  Session sharing is ready.\n'
if test -f "$backup/codex.fish"; or test -L "$backup/codex-entry"
    printf '  Previous setup saved to %s\n' "$backup"
end
