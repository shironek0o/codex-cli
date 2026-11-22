function codex --description 'Codex native interface with shared local history'
    set -l codex_home "$HOME/.codex"
    if test -n "$CODEX_HOME"
        set codex_home "$CODEX_HOME"
    end
    set -l auto_switch_off "$codex_home/session-sharing-auto-switch-off"

    if test "$argv[1]" = auto-switch
        set -l action "$argv[2]"
        if set -q argv[3]; or not contains -- "$action" '' status on off
            printf 'Usage: codex auto-switch [status|on|off]\n' >&2
            return 2
        end
        switch "$action"
            case off
                command mkdir -p -m 700 -- "$codex_home"
                and command touch -- "$auto_switch_off"
                or return
            case on
                command rm -f -- "$auto_switch_off"
                or return
        end
        if command printenv CODEX_AUTO_SWITCH_OFF >/dev/null
            printf 'Automatic quota checks and account switching: off (CODEX_AUTO_SWITCH_OFF is set)\n'
        else if test -f "$auto_switch_off"
            printf 'Automatic quota checks and account switching: off\n'
        else
            printf 'Automatic quota checks and account switching: on\n'
        end
        return 0
    end

    set -l upstream (command -s codex)
    set -l adapter "$HOME/.local/share/codex-session-sharing/codex-session-sharing"
    if not test -x "$adapter"
        command "$upstream" $argv
        return $status
    end
    if test -f "$auto_switch_off"
        set --function --export CODEX_AUTO_SWITCH_OFF 1
    end
    "$adapter" --binary "$upstream" -- $argv
end
