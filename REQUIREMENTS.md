# flick: requirements

flick is a zsh autocomplete for macOS and Linux that replaces Fig. Terms are defined in [GLOSSARY.md](GLOSSARY.md), and the reasons behind the main decisions are in [docs/adr/](docs/adr/).

## Goals, in priority order

1. **Speed and low memory**, kept up over days of uptime.
2. **Coverage**: as many CLIs as possible.
3. **Extensibility**: anyone can add a CLI.
4. **UX and UI**, including Themes.
5. **Cross-platform**: zsh on macOS and Linux (x86_64 and arm64).

## Soft budgets

Every change is measured against these with `flick bench`. They are targets, to be tightened or relaxed once we have real numbers.

| Metric | Budget |
|---|---|
| Keypress → Menu update | ≤ 10 ms |
| Added shell startup time | ≤ 20 ms |
| Total memory (background process plus per-shell overhead) | ≤ 15 MB, flat over days |

## Architecture (see ADRs)

- **Menu drawn inline** below the prompt using ZLE, not as a floating window ([0001](docs/adr/0001-inline-menu-not-floating-window.md)).
- **One shared background process**, written in Rust, serves all shells over a unix socket ([0002](docs/adr/0002-one-shared-background-process.md), [0005](docs/adr/0005-rust.md)).
  - It starts automatically when the first shell needs it and exits when it has been idle with no shells connected.
  - It restarts silently after a crash.
  - If it is unavailable, the shell keeps working with no Menu. No error, no hang.
  - Keep the socket path short (macOS limits it to ~104 bytes).
- **Fig specs** (withfig/autocomplete, MIT, 715 CLIs) are converted once into `specs/` and maintained there from then on. All of them are committed ([0004](docs/adr/0004-vendor-fig-specs.md)).
- **Specs load lazily**: a CLI's Spec loads when that CLI is first typed, and unused Specs are evicted ([0006](docs/adr/0006-load-specs-lazily.md)).
- **No JavaScript runtime** ([0003](docs/adr/0003-no-javascript-runtime-for-fig-specs.md)). Spec parts that need code fall back to Native completion, and common patterns are rewritten as plain settings.

## Functional requirements

### Sources

- All five Sources, each switchable on or off and weighted in config: **Spec**, **Generator**, **Files**, **History**, **Native completion**.
- **Presets** for common combinations, at least `all` and `history-only`.
- **Native completion** is used for CLIs with no Spec, and wherever a Spec needs code flick doesn't run. When flick has nothing to suggest, Tab hands over to whatever Tab did before flick loaded (zsh's own completion, fzf-tab, ...), drawn by zsh itself. Showing Native completion results inside flick's Menu (capturing them as fzf-tab does) is deferred (Q32).
- At the command position, every executable on `PATH` is a Suggestion, alongside CLIs that have a Spec.

### Generators

- A Generator runs a command and reads its output using declarative parsing settings: split the output into lines, capture name and description with a pattern, or read a field from JSON. There is no JavaScript (ADR 0003).
- Hand-port the Generators of the most-used CLIs first: git, docker, brew, npm. Other Generators fall back to Native completion until they are ported. The format is documented in `specs/README.md`.
- Generator results rank above files and static values.
- Generators never block typing. Static Suggestions show immediately, and Generator results fill in when they arrive.
- Each Generator has its own timeout and a short-lived cache.
- Generators that use the network can be disabled per CLI.

### History

- flick keeps its own History. Each History entry stores the command, directory, exit code and timestamp.
- On first run, flick imports `~/.zsh_history` once. Imported entries have no directory or exit code.
- flick never records commands that start with a space, or commands matching configurable patterns. The defaults are patterns containing `password`, `TOKEN=` and `SECRET`.
- Nothing leaves the machine. No telemetry.

### Ranking

1. Exact prefix matches rank above fuzzy matches.
2. Frecency: Suggestions used often and recently rank higher.
3. A boost for commands previously run in the current directory.

Menu order: the best History line comes first, so it is also the Ghost text. Then come Spec Suggestions, ordered by how often the user has run each one, then up to 3 more History lines. A History line replaces the whole line; any other Suggestion replaces only the current word.

### Display

- **Ghost text**: the top Suggestion, dimmed, after the cursor. It replaces `zsh-autosuggestions`.
- **Menu**: up to 8 rows, each showing an icon (optional, needs a Nerd Font), the name and a dimmed description. It scrolls within those 8 rows.
- **Trigger modes**: as-you-type and on-demand (Tab), switchable in config. The default is as-you-type: the spike measured ~0.4 ms per keystroke (see `spike/RESULTS.md`).
- When the prompt is at the bottom of the window, scroll up to make room for the Menu. If the window is shorter than ~10 rows, show only Ghost text.

### Keys (all rebindable)

| Key | Menu open | Menu closed |
|---|---|---|
| ↑ / ↓ | Move the selection | Normal history |
| Tab | Accept the selected Suggestion | Open the Menu |
| → | Accept the Ghost text | Accept the Ghost text |
| Enter | Run exactly what was typed. Never accepts a Suggestion | Run |
| Esc | Close the Menu | n/a |

### Customisation

- **Config**: `~/.config/flick/config.toml`.
- **Themes**: colours, borders and icons only, never layout. The default Theme uses the terminal's own ANSI palette, so it follows light and dark mode. One alternative Theme ships with v1.
- **User specs**: YAML files in `~/.config/flick/specs/`, using the same structure as converted Fig specs.

### Shell integration

- Load flick with one `source` line, after oh-my-zsh and `compinit` and before `zsh-syntax-highlighting`.
- Print nothing at startup, so p10k's instant prompt keeps working.
- If `zsh-autosuggestions` is also loaded, warn once and don't draw Ghost text.
- Leave fzf's Ctrl-R alone.

## v1 scope

**In:** everything above.

**Later:**

- Generating Specs from `--help` output
- An embedded JS engine, e.g. QuickJS
- A Homebrew formula and plugin-manager listings
- Syncing History across machines

## Install (personal phase)

Clone the repo, build it, and add `source …/flick.zsh` to `.zshrc`. Releases are a single self-contained binary per platform, so putting flick on a Linux server means copying one file.

## Assumed defaults (not discussed; flag any you disagree with)

- **Aliases** resolve to their target's Spec, so `g` gets git's Suggestions when `alias g=git`.
- **Prefixes** are skipped before finding the command: `sudo`, `env`, `time`, `nohup` and `VAR=value` assignments.
- **Each segment** after `|`, `&&`, `||` and `;` is completed as its own command.
- **Commands spanning several lines** (backslash continuation, here-docs) get Ghost text only, no Menu.
