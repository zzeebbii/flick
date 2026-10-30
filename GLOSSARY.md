# flick

flick is a zsh autocomplete that suggests completions inline in the terminal while you type, as a replacement for Fig.

## Language

### What the user sees

**Menu**:
The list of Suggestions drawn inline in the terminal, directly below the prompt line.
_Avoid_: Popup, dropdown, overlay

**Ghost text**:
The single top Suggestion shown dimmed after the cursor on the prompt line itself.
_Avoid_: Autosuggestion, inline hint

**Suggestion**:
One candidate completion of the current word or line, offered in the Menu or as Ghost text.
_Avoid_: Completion, candidate, item

**Trigger mode**:
Whether the Menu opens on every keystroke ("as-you-type") or only when the user presses Tab ("on-demand").

### Where Suggestions come from

**Source**:
A provider of Suggestions. There are five: Spec, Generator, Files, History and Native completion.

**Spec**:
A declarative description of one CLI's subcommands, options and arguments.
_Avoid_: Completion spec, definition, schema

**Generator**:
The part of a Spec that produces values at runtime by running a command, e.g. listing git branches.
_Avoid_: Dynamic completion, provider

**Native completion**:
zsh's own built-in completion functions, used for CLIs that have no Spec and wherever a Spec needs code we don't run.
_Avoid_: compsys fallback, zsh completion

**History entry**:
One command the user ran, with the directory it ran in, its exit code and a timestamp.
_Avoid_: History line, command log

**Preset**:
A named combination of enabled Sources and their weights, such as `history-only` or `all`.
_Avoid_: Mode, profile

### Customisation

**Theme**:
The colours, borders and icons of the Menu and Ghost text. A Theme never controls layout.
_Avoid_: Skin, style

**User spec**:
A Spec that a user writes and drops into their config directory, as opposed to one shipped with the tool.
_Avoid_: Custom spec, plugin
