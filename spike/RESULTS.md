# Spike results: shell ↔ background process latency

Measured 2026-10-30 on an Apple Silicon Mac with zsh 5.9. Reproduce with `zsh spike/bench.zsh`, after building with `cargo build --release` in `spike/`.

The setup is a Rust process (std library only) that loads `~/.zsh_history` (8k lines) and does prefix plus fuzzy matching. A zsh plugin keeps one unix socket open per shell and draws Ghost text and an 8-row Menu using `POSTDISPLAY` and `region_highlight`.

| Measurement | p50 | p95 | max | Budget |
|---|---|---|---|---|
| Socket round-trip, shell → process → shell | 0.09 ms | 0.20 ms | 0.28 ms | — |
| Per keystroke inside ZLE (query and draw) | 0.40 ms | 0.82 ms | 1.28 ms | ≤ 10 ms |
| Added shell startup time | 2.6 ms | 3.1 ms | 4.9 ms | ≤ 20 ms |
| Background process memory (RSS) | 3.2 MB | | | ≤ 15 MB |

**Conclusion:** the architecture in ADR 0002 fits the budgets with room to spare, so as-you-type is the default Trigger mode.

**Not measured yet:**
- Terminal paint time in iTerm and Ghostty.
- Memory once 715 Specs are loaded.
- Linux.

# Spike results: Spec conversion and memory

The converter is `tools/fig-import/convert.mjs`. It converts all 1,478 spec files in about 1.4 s, producing 80 MB of JSON. The memory figures below come from `spike/src/bin/specmem.rs`, run under `/usr/bin/time -l`:

| Specs loaded | Max memory (RSS) |
|---|---|
| None (baseline) | 2.1 MB |
| git | 3.8 MB |
| The 11 most-used CLIs in the user's history | 4.8 MB |
| All 1,475 | 169 MB |

Three specs (`vr`, `zellij`, `uv`) don't fit the spike's typed schema yet.

**Conclusion:** load Specs lazily (ADR 0006).

**Generators:** 5,193 of 5,316 (98%) relied on JavaScript. 3,538 of those still have their command as plain data (398 distinct commands across all Specs), so only the step that reads the output is missing. In the most-used CLIs, the number of distinct Generators is small: git 14, docker 18, brew 9, npm 2.
