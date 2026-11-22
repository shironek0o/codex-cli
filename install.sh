#!/bin/sh
set -eu

if [ "$#" -eq 0 ]; then
    settings_only=false
elif [ "$#" -eq 1 ] && [ "$1" = --settings-only ]; then
    settings_only=true
else
    echo 'Usage: ./install.sh [--settings-only]' >&2
    exit 2
fi

CDPATH= cd -- "$(dirname -- "$0")"
codex_home="${CODEX_HOME:-$HOME/.codex}"
codex_auth_version=0.4.0
theme=themes/solarized-osaka.tmTheme
if [ ! -r AGENTS.md ]; then
    echo 'Missing AGENTS.md.' >&2
    exit 1
fi
if [ ! -r "$theme" ]; then
    echo 'Missing Solarized Osaka theme file.' >&2
    exit 1
fi

install_codex_auth_release() (
    if [ "$(uname -s):$(uname -m)" != Linux:x86_64 ]; then
        echo 'The npm-free codex-auth installer requires x86_64 Linux.' >&2
        exit 1
    fi
    download_dir=$(mktemp -d)
    archive="$download_dir/codex-auth-Linux-X64.tar.gz"
    trap 'rm -f -- "$archive" "$download_dir/codex-auth"; rmdir -- "$download_dir"' EXIT
    release="https://github.com/Loongphy/codex-auth/releases/download/v$codex_auth_version"
    curl -fsSL --output "$archive" "$release/${archive##*/}"
    # SHA-256 from the upstream v0.4.0 release assets.
    if ! printf '%s  %s\n' e74c45a2bc6b97a0696660fcae1c7a49fb5590b81851fbd53e1fc07e549cb5ce "$archive" | sha256sum --check --status; then
        echo 'The codex-auth download failed its SHA-256 check.' >&2
        exit 1
    fi
    tar -xzf "$archive" -C "$download_dir" -- codex-auth
    "$download_dir/codex-auth" --version >/dev/null
    install -Dm755 -- "$download_dir/codex-auth" "$1"
)

if [ "$settings_only" = false ]; then
    if ! command -v codex >/dev/null 2>&1 || ! command -v fish >/dev/null 2>&1; then
        echo 'Full installation requires Codex CLI and Fish. Run ./install.sh --settings-only to install the settings.' >&2
        exit 1
    fi

    echo 'Set up the Codex account switcher'
    if command -v codex-auth >/dev/null 2>&1; then
        codex-auth --version
    else
        prefix=$HOME/.local
        if [ ! -x "$prefix/bin/codex-auth" ]; then
            if command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1; then
                npm install --global --prefix "$prefix" "@loongphy/codex-auth@$codex_auth_version"
            else
                install_codex_auth_release "$prefix/bin/codex-auth"
            fi
        fi
        "$prefix/bin/codex-auth" --version
        fish -c 'if not contains -- $argv[1] $PATH; fish_add_path --universal --append $argv[1]; end' "$prefix/bin"
    fi

    echo 'Set up shared Codex sessions'
    fish --no-config ./session-sharing/install.fish
fi

echo 'Install Codex instructions and the Solarized Osaka theme'
agents_destination="$codex_home/AGENTS.md"
if ! cmp -s -- AGENTS.md "$agents_destination"; then
    install -Dm644 -T --backup=numbered -- AGENTS.md "$agents_destination"
fi
install -Dm644 -- "$theme" "$codex_home/$theme"
echo 'Install Codex skills'
skill_root="$HOME/.agents/skills"
codex_skill_root="$codex_home/skills"
if [ -d "$skill_root/inkdrop-plan" ] && [ ! -e "$skill_root/inkdrop" ]; then
    mv -T -- "$skill_root/inkdrop-plan" "$skill_root/inkdrop"
fi
for root in "$skill_root" "$codex_skill_root"; do
    if [ ! -e "$root/algo-tutor" ] && [ -f "$root/tutor/SKILL.md" ] &&
        cmp -s -- skills/algo-tutor/LICENSE "$root/tutor/LICENSE" &&
        grep -Fxq '# Algorithm Problem Tutor' "$root/tutor/SKILL.md"; then
        mv -T -- "$root/tutor" "$root/algo-tutor"
    fi
done
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
printf '\nSetup complete.\n'
if [ "$settings_only" = false ]; then
    echo '  In a new Fish terminal: run codex-auth switch, then codex resume.'
fi
cat <<'EOF'
  In Codex: open /theme, choose solarized-osaka (custom), and press Enter.
  In Codex: type $ to browse your skills, or type $inkdrop to plan in Inkdrop.
EOF
