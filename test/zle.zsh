#!/bin/zsh -f
# Check the zsh side against a real background process: word splitting, aliases, the Menu and
# its keys. Calls the widget functions directly (no terminal needed).
# Run: zsh test/zle.zsh   (after cargo build --release)
emulate -L zsh
root=${0:A:h:h}
tmp=$(mktemp -d)
trap 'pkill -f "flick serve $tmp/s.sock"; rm -rf $tmp' EXIT
export FLICK_HISTORY=$tmp/history.tsv FLICK_HISTFILE=$tmp/none  # never touch the real History
source $root/flick.zsh
FLICK_SOCK=$tmp/s.sock
alias gco='git checkout'
zle() { [[ $1 == .forward-char ]] }  # stand-in for the zle builtin outside the line editor
LINES=40 COLUMNS=120
fails=0

_flick_connect; for i in {1..50}; do [[ -S $FLICK_SOCK ]] && break; sleep 0.02; done

type_line() { LBUFFER=$1 BUFFER=$1 CURSOR=$#1; _flick_query; _flick_render }
expect() {  # expect <description> <condition>
  if eval $2; then print "ok    $1"; else print "FAIL  $1  (LBUFFER='$LBUFFER' POSTDISPLAY=${(q+)POSTDISPLAY})"; fails=1; fi
}
accepts() {  # accepts <typed line> <line after Right arrow>
  local want=$2
  type_line $1; _flick_right
  expect "Right arrow: '$1' -> '$want'" '[[ $LBUFFER == $want ]]'
}

accepts 'git ch'              'git checkout'
accepts 'gco --det'           'gco --detach'
accepts 'ls | git stat'       'ls | git status'
accepts 'FOO=1 brew ins'      'FOO=1 brew install'
accepts 'echo hi && git comm' 'echo hi && git commit'
accepts 'nosuchcmd x'         'nosuchcmd x'

type_line 'git ch'
expect 'Ghost text, then Menu rows'   '[[ ${POSTDISPLAY%%$'"'\n'"'*} == eckout && ${#${(f)POSTDISPLAY}} -ge 3 ]]'
expect 'Menu is open'                 '(( FLICK_OPEN ))'
_flick_down
expect 'Down selects the 2nd row'     '(( FLICK_SEL == 2 ))'
expect 'Ghost text follows selection' '[[ $POSTDISPLAY == erry-pick$'"'\n'"'* ]]'
_flick_up; _flick_up
expect 'Up wraps to the last row'     '(( FLICK_SEL == $#FLICK_INSERT ))'
_flick_down; _flick_tab
expect 'Tab takes selection + space'  '[[ $LBUFFER == "git checkout " ]]'

type_line 'git commit --am'; _flick_esc
expect 'Esc closes the Menu'          '(( ! FLICK_OPEN )) && [[ -z $POSTDISPLAY ]]'
_flick_tab
expect 'Tab reopens a closed Menu'    '(( FLICK_OPEN ))'

type_line 'git '; for i in {1..12}; do _flick_down; done
expect 'Menu scrolls past 8 rows'     '(( FLICK_SEL > 8 )) && [[ $POSTDISPLAY == *"($FLICK_SEL/"* ]]'

LINES=8; type_line 'git ch'
expect 'Short window: Ghost text only' '[[ $POSTDISPLAY == eckout ]]'
_flick_tab
expect 'Short window: Tab still takes Ghost text' '[[ $LBUFFER == "git checkout " ]]'
LINES=40

# Regression: $'\n' in the line must not split one request into two and desync every answer after.
type_line "git commit -m \$'a\\nb' "; type_line 'git ch'
expect "Newline in a word doesn't desync answers" '[[ $POSTDISPLAY == eckout$'"'\n'"'* ]]'

FLICK_NO_GHOST=1; type_line 'git ch'
expect 'No Ghost text when zsh-autosuggestions is loaded' '[[ $POSTDISPLAY == $'"'\n'"'* ]]'
FLICK_NO_GHOST=0

# History: a recorded command leads the Menu as a whole line and becomes the Ghost text.
FLICK_CMD='git checkout main'; true; _flick_precmd
FLICK_CMD='git checkout main'; true; _flick_precmd
FLICK_CMD='git checkout broken'; false; _flick_precmd
type_line 'git ch'
expect 'History line leads the Menu'      '[[ ${FLICK_KIND[1]} == l && ${FLICK_INSERT[1]} == "git checkout main" ]]'
expect 'Ghost text completes the line'    '[[ $POSTDISPLAY == "eckout main"$'"'\n'"'* ]]'
expect 'Only-failed command left out'     '(( ! ${FLICK_INSERT[(I)git checkout broken]} ))'
_flick_right
expect 'Right takes the whole line'       '[[ $LBUFFER == "git checkout main" ]]'
type_line 'gi'
expect 'Used command outranks the alphabet' '[[ ${FLICK_INSERT[2]} == git ]]'
FLICK_CMD=' git push --secret'; true; _flick_precmd
type_line 'git pu'
expect 'Leading space is not recorded'    '[[ ${FLICK_KIND[1]} != l ]]'
grep -q "git checkout main" $FLICK_HISTORY
expect 'History is written to disk'       '(( $? == 0 ))'

exit $fails
