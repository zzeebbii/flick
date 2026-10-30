# flick

A fast zsh autocomplete for macOS and Linux that replaces Fig. See [REQUIREMENTS.md](REQUIREMENTS.md) and [GLOSSARY.md](GLOSSARY.md).

## Try it

```sh
cargo build --release
zsh -f                        # a clean shell; zsh-autosuggestions would clash
source ./flick.zsh
```

Type `git ch`: ↑/↓ choose a Suggestion, Tab takes it (plus a space), → takes the Ghost text, Esc closes the Menu.

## Develop

```sh
cargo test --release            # Spec walking, plus a check that every Spec in specs/ loads
zsh test/zle.zsh                # zsh side against a real background process
expect test/keys.exp            # real keystrokes in a real terminal
flick complete git checkout --  # print Suggestions for a line; the last word is the one being typed (no Generators)
node tools/fig-import/port-generators.mjs  # apply tools/fig-import/ports.json to specs/
```
