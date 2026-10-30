# flick

A fast zsh autocomplete for macOS and Linux that replaces Fig. See [REQUIREMENTS.md](REQUIREMENTS.md) and [GLOSSARY.md](GLOSSARY.md).

## Try it

```sh
cargo build --release
zsh -f                        # a clean shell; zsh-autosuggestions would clash
source ./flick.zsh
```

Type `git ch`: ↑/↓ choose a Suggestion, Tab takes it (plus a space), → takes the Ghost text, Esc closes the Menu.

## Configure

```sh
mkdir -p ~/.config/flick
flick config example > ~/.config/flick/config.toml   # every setting, commented, with its default
flick config check                                   # validate after editing
```

- **Themes**: `theme = "default"` (your terminal's colours) or `"nerd"` (icons; needs a Nerd Font). For your own, put a file in `~/.config/flick/themes/<name>.toml` with any of `ghost`, `name`, `description`, `selected`, `history` (zsh highlight specs such as `fg=6,bold`) and an `[icons]` table (`history`, `command`, `option`, `file`, `folder`).
- **User specs**: put `<cli>.yaml` (or `.json`) in `~/.config/flick/specs/`. These use the same structure as `specs/` (see `specs/README.md`) and replace a shipped Spec of the same name.

New shells pick up changes; the background process re-reads `config.toml` as soon as it changes.

## Develop

```sh
cargo test --release            # Spec walking, plus a check that every Spec in specs/ loads
zsh test/zle.zsh                # zsh side against a real background process
expect test/keys.exp            # real keystrokes in a real terminal
flick complete git checkout --  # print Suggestions for a line; the last word is the one being typed (no Generators)
flick bench                     # speed and memory against the budgets in REQUIREMENTS.md
node tools/fig-import/port-generators.mjs  # apply tools/fig-import/ports.json to specs/
```
