# Draw the Menu inside the terminal, not as a floating window

Fig drew its popup as a separate macOS window positioned using the Accessibility API and an input method. Neither iTerm2 nor Ghostty offers an API to put a popup at the cursor, and there is no Linux equivalent. We draw the Menu inline below the prompt using ZLE, the zsh line editor. That is the only approach that works the same on macOS, Linux and over SSH, and it needs no OS permissions.
