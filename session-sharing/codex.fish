function codex --description 'Codex native interface with shared local history'
    set -l adapter "$HOME/.local/share/codex-session-sharing/codex-session-sharing"
    if contains -- "$argv[1]" auto-switch auto-resume
        if not test -x "$adapter"
            printf 'Error: the session-sharing adapter is missing. Rerun the installer.\n' >&2
            return 1
        end
        set -l option --$argv[1]
        set -e argv[1]
        "$adapter" "$option" $argv
        return $status
    end

    set -l upstream (command -s codex)
    if not test -x "$adapter"
        command "$upstream" $argv
        return $status
    end
    "$adapter" --binary "$upstream" -- $argv
end
