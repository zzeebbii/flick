# Don't run the JavaScript in Fig specs

Running Fig's JavaScript as written would mean embedding a JS engine or bundling Node.js, as inshellisense does, and that conflicts with the memory goal. We convert only the declarative parts of each spec.

The import (2026-10-30) showed that 98% of Generators (5,193 of 5,316) relied on JavaScript. In most of them, 3,538 in total, the command is still plain data, and only the step that reads its output is code. That step comes in 576 distinct versions across all specs.

So Specs get a small set of declarative output-parsing settings instead: split the output into lines, capture name and description with a pattern, or read a field from JSON. We hand-port Generators to those settings for the CLIs the user runs most, starting with git (14 distinct Generators), docker (18), brew (9) and npm (2). Every other Generator stays `needsCode: true` and falls back to Native completion.

If coverage turns out noticeably worse than Fig's, embedding a small engine such as QuickJS is the upgrade path.
