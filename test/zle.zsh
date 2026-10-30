#!/bin/zsh -f
# Check the zsh side against a real background process: word splitting, aliases, and
# accepting the top Suggestion. Calls the widget functions directly (no terminal needed).
# Run: zsh test/zle.zsh   (after cargo build --release)
emulate -L zsh
root=${0:A:h:h}
tmp=$(mktemp -d)
trap 'pkill -f "flick serve $tmp/s.sock"; rm -rf $tmp' EXIT
source $root/flick.zsh
FLICK_SOCK=$tmp/s.sock
alias gco='git checkout'
zle() { [[ $1 == .forward-char ]] }  # stand-in for the zle builtin outside the line editor
fails=0

_flick_connect; for i in {1..50}; do [[ -S $FLICK_SOCK ]] && break; sleep 0.02; done

accepts() {  # accepts <typed line> <line after Right arrow>
  LBUFFER=$1 BUFFER=$1 CURSOR=$#1 POSTDISPLAY=
  _flick_draw; _flick_accept
  if [[ $LBUFFER == $2 ]]; then print "ok    '$1' -> '$LBUFFER'"
  else print "FAIL  '$1': want '$2', got '$LBUFFER'"; fails=1; fi
}

accepts 'git ch'              'git checkout'
accepts 'gco --det'           'gco --detach'
accepts 'ls | git stat'       'ls | git status'
accepts 'FOO=1 brew ins'      'FOO=1 brew install'
accepts 'echo hi && git comm' 'echo hi && git commit'
accepts 'nosuchcmd x'         'nosuchcmd x'

LBUFFER='git ch' BUFFER='git ch' CURSOR=6; _flick_draw
[[ $POSTDISPLAY == eckout$'\n'* ]] && print "ok    Ghost text 'eckout' plus Menu" || { print "FAIL  POSTDISPLAY=${(q+)POSTDISPLAY}"; fails=1 }
exit $fails
