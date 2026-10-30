# One shared background process serves all shells

Starting a program on every keypress costs about 5ms on macOS (measured), which is half the speed budget before any work is done. A request over a unix socket to an already-running process costs about 0.02ms. We run one background process shared by every open shell, rather than one per shell or pure zsh, so memory stays flat no matter how many tabs are open. It starts when the first shell needs it, exits when it has been idle with no shells connected, and the shell keeps working without a Menu whenever it is unavailable.

## Considered Options

- Starting a program per keypress: too slow (see above).
- One helper process per shell: memory grows with the number of tabs.
- Pure zsh: every shell holds its own copy of the specs, and matching in zsh is slow.
