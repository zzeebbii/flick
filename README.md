# flick

A fast zsh autocomplete for macOS and Linux that replaces Fig. See [REQUIREMENTS.md](REQUIREMENTS.md) and [GLOSSARY.md](GLOSSARY.md).

## Try it

```sh
cargo build --release
zsh -f                        # a clean shell; zsh-autosuggestions would clash
source ./flick.zsh
```

Type `git ch` and press → to accept.

## Develop

```sh
cargo test --release            # Spec walking, plus a check that every Spec in specs/ loads
zsh test/zle.zsh                # zsh side against a real background process
flick complete git checkout --  # print Suggestions for a line; the last word is the one being typed
```
