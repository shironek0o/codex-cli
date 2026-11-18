#!/usr/bin/fish --no-config

set -l integration "$HOME/.config/fish/functions/codex.fish"
if test -f "$integration"
    if not string match -q '*codex-session-sharing*' <"$integration"
        printf 'Error: codex.fish belongs to a different integration; nothing was removed.\n' >&2
        exit 1
    end
    rm "$integration"
    or exit 1
end
printf 'Removed shell integration. In existing Fish terminals, run: functions -e codex\n'
