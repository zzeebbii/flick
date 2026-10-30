# Specs

These were converted once from [withfig/autocomplete](https://github.com/withfig/autocomplete) at commit `aef52ac` (2025-05-05), using `tools/fig-import/convert.mjs`. They are MIT licensed; see `LICENSE-withfig`. From here on they are maintained in this repo (ADR 0004).

## Generators

A Generator runs a command and turns its output into Suggestions. It runs in the directory you're typing in, in the background, and its result is cached.

| Field | Meaning |
|---|---|
| `script` | The command, as an argument list (`["git", "branch"]`), or a string run with `sh -c` |
| `splitOn` | Split the output on this string instead of on lines |
| `parse.pattern` | Regex applied to each line. Lines that don't match are dropped. The named groups `name` and `description` set those fields; without a `name` group, the whole match is the name |
| `parse.exclude` | Regex; lines that match it are dropped |
| `parse.description` | Description used when the pattern has no `description` group |
| `parse.json` | Read the output as JSON instead: one document, or one per line. `path` is a dot path into the document, `name` and `description` are fields, and `$key` / `$value` read an object's keys and values |
| `cache.ttl` | Milliseconds a result stays fresh. Default 5000. After that, the old result is still shown while a new one is fetched |
| `scriptTimeout` | Milliseconds before the command is killed. Default 5000 |
| `template` | `filepaths` or `folders`: list files instead of running a command |
| `needsCode: true` | Fig ran JavaScript here. The Generator is skipped and falls back to Native completion until it is ported |

Ports of Fig's JavaScript Generators live in `tools/fig-import/ports.json`. To apply them, run `node tools/fig-import/port-generators.mjs`.

Example (git branches):

```json
{ "script": ["git", "branch", "--no-color"],
  "parse": { "pattern": "^[*+ ] (?P<name>\\S+)", "exclude": "HEAD", "description": "Branch" } }
```
