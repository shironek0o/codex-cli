function codex --description 'Codex native interface with shared local history'
    set -l upstream (command -s codex)
    set -l adapter "$HOME/.local/share/codex-session-sharing/codex-session-sharing"
    if not test -x "$adapter"
        command "$upstream" $argv
        return $status
    end
    "$adapter" --binary "$upstream" -- $argv
end
