#!/bin/sh
set -eu

CDPATH= cd -- "$(dirname -- "$0")"
theme=themes/solarized-osaka.tmTheme
if [ ! -r "$theme" ]; then
    echo 'Missing Solarized Osaka theme file.' >&2
    exit 1
fi

echo '[1/4] Set up the Codex account switcher'
if command -v codex-auth >/dev/null 2>&1; then
    codex-auth --version
else
    prefix=$HOME/.local
    if [ ! -x "$prefix/bin/codex-auth" ]; then
        if ! command -v npm >/dev/null 2>&1; then
            echo 'Install Node.js and npm, then run this installer again.' >&2
            exit 1
        fi
        npm install --global --prefix "$prefix" @loongphy/codex-auth@0.4.0
    fi
    "$prefix/bin/codex-auth" --version
    fish -c 'if not contains -- $argv[1] $PATH; fish_add_path --universal --append $argv[1]; end' "$prefix/bin"
fi

echo '[2/4] Set up shared Codex sessions'
fish --no-config ./session-sharing/install.fish
echo '[3/4] Install the Solarized Osaka theme'
install -Dm644 -- "$theme" "${CODEX_HOME:-$HOME/.codex}/$theme"
echo '[4/4] Install Codex skills'
skill_root="$HOME/.agents/skills"
codex_skill_root="${CODEX_HOME:-$HOME/.codex}/skills"
if [ -d "$skill_root/inkdrop-plan" ] && [ ! -e "$skill_root/inkdrop" ]; then
    mv -T -- "$skill_root/inkdrop-plan" "$skill_root/inkdrop"
fi
for skill in skills/*; do
    [ -f "$skill/SKILL.md" ] || continue
    name=${skill##*/}
    destination="$skill_root/$name"
    if [ ! -f "$destination/SKILL.md" ] && [ -f "$codex_skill_root/$name/SKILL.md" ]; then
        destination="$codex_skill_root/$name"
    fi
    mkdir -p -- "$destination"
    cp -a -- "$skill/." "$destination/"
    printf '  Installed %s\n' "$name"
done
cat <<'EOF'

Setup complete.
  In a new Fish terminal: run codex-auth switch, then codex resume.
  In Codex: open /theme, choose solarized-osaka (custom), and press Enter.
  In Codex: type $ to browse your skills, or type $inkdrop to plan in Inkdrop.
EOF
