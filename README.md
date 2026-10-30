# flick

A fast zsh autocomplete for macOS and Linux that replaces Fig. See [REQUIREMENTS.md](REQUIREMENTS.md) and [GLOSSARY.md](GLOSSARY.md).

## Install (from this repo)

```sh
cargo build --release
ln -s "$PWD/target/release/flick" ~/.local/bin/flick   # the `flick` command; any directory on PATH works
echo "source $PWD/flick.zsh" >> ~/.zshrc               # near the end; remove zsh-autosuggestions from plugins
```

To try it without touching `.zshrc`: `zsh -f`, then `source ./flick.zsh`.

Type `git ch`: ↑/↓ choose a Suggestion, Tab takes it (plus a space), → takes the Ghost text, Esc closes the Menu.

## Configure

```sh
mkdir -p ~/.config/flick
flick config example > ~/.config/flick/config.toml   # every setting, commented, with its default
flick config check                                   # validate after editing
```

- **Themes**: `theme = "default"` (your terminal's colours), `"panel"` (a dark card, like a popup; needs 24-bit colour, which iTerm and Ghostty have) or `"nerd"` (icons; needs a Nerd Font).
  - For your own, put a file in `~/.config/flick/themes/<name>.toml`. Any key you leave out keeps the default Theme's value.
  - Each value is a zsh highlight spec: `fg=` / `bg=` with a colour name, a 0–255 number or `#rrggbb`, plus `bold`, `underline` or `standout`, comma-separated (e.g. `fg=#7aa2f7,bold`).
  - Keys: `ghost`, `name` (commands, subcommands, fixed values), `option`, `file`, `folder`, `value` (Generator results such as branches), `history`, `description`, `selected` (the whole selected row), `match` (the typed part of each name), `panel` (background of the whole Menu), and an `[icons]` table (`history`, `command`, `option`, `file`, `folder`).
- **User specs**: put `<cli>.yaml` (or `.json`) in `~/.config/flick/specs/`. These use the same structure as `specs/` (see `specs/README.md`) and replace a shipped Spec of the same name.

New shells pick up changes; the background process re-reads `config.toml` as soon as it changes.

## Develop

```sh
cargo test --release            # Spec walking, plus a check that every Spec in specs/ loads
zsh test/zle.zsh                # zsh side against a real background process
expect test/keys.exp            # real keystrokes in a real terminal
expect test/cursor.exp          # align = "cursor": cursor position query and typed-ahead keys
flick complete git checkout --  # print Suggestions for a line; the last word is the one being typed (no Generators)
flick bench                     # speed and memory against the budgets in REQUIREMENTS.md
node tools/fig-import/port-generators.mjs  # apply tools/fig-import/ports.json to specs/
```
