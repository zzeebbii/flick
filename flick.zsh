# flick: zsh side. Source this from .zshrc after oh-my-zsh/compinit, before zsh-syntax-highlighting.
# Must print nothing at startup (p10k instant prompt).
zmodload zsh/net/socket zsh/zselect zsh/datetime 2>/dev/null || return
autoload -Uz add-zle-hook-widget add-zsh-hook

typeset -g FLICK_SOCK=${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/flick-$UID.sock
(( $#FLICK_SOCK < 100 )) || FLICK_SOCK=/tmp/flick-$UID.sock  # macOS caps socket paths at ~104 bytes
typeset -g FLICK_BIN=${0:A:h}/target/release/flick
[[ -x $FLICK_BIN ]] || FLICK_BIN=${commands[flick]:-}
typeset -g FLICK_TRIGGER=${FLICK_TRIGGER:-typing}  # typing | tab (Trigger mode)
typeset -gi FLICK_ROWS=${FLICK_ROWS:-8}

typeset -g FLICK_FD=
typeset -ga FLICK_KIND FLICK_INSERT FLICK_DESC  # current Suggestions; kind w = word, l = whole line
typeset -g FLICK_WORD=               # the word they replace
typeset -gi FLICK_SEL=1              # selected Suggestion
typeset -gi FLICK_OPEN=0             # Menu is showing
typeset -g FLICK_LAST=               # LBUFFER the Suggestions were made for
typeset -gi FLICK_NO_GHOST=0         # zsh-autosuggestions owns Ghost text
typeset -g FLICK_CMD=                # command line now running, for History
typeset -gi FLICK_WATCHING=0         # zle -F handler installed on FLICK_FD
# Whatever Tab did before flick (expand-or-complete, fzf-tab, ...): Native completion, used
# when flick has nothing to suggest.
typeset -g FLICK_NATIVE_TAB=${${(z)"$(bindkey '^I')"}[2]:-expand-or-complete}  # `"^I" widget`
[[ $FLICK_NATIVE_TAB == (_flick_tab|undefined-key) ]] && FLICK_NATIVE_TAB=expand-or-complete  # sourced twice

_flick_connect() {
  [[ -n $FLICK_FD ]] && return 0
  if ! zsocket $FLICK_SOCK 2>/dev/null; then
    [[ -n $FLICK_BIN ]] || return 1
    FLICK_HISTFILE=${FLICK_HISTFILE:-$HISTFILE} $FLICK_BIN serve $FLICK_SOCK &>/dev/null &!
    # Only the first keystroke after flick starts pays this; it is up in a few ms.
    local i
    for i in {1..10}; do
      zselect -t 1  # 10 ms, without forking sleep
      zsocket $FLICK_SOCK 2>/dev/null && break
    done || return 1
  fi
  FLICK_FD=$REPLY
  return 0
}

# A Generator is running; redraw when it finishes (`!`) while the shell is idle.
# Watched only while waiting, so an idle shell isn't woken for nothing.
_flick_watch()   { (( FLICK_WATCHING )) || { zle -F -w $FLICK_FD _flick_on_data 2>/dev/null && FLICK_WATCHING=1 } }
_flick_unwatch() { (( FLICK_WATCHING )) && zle -F $FLICK_FD 2>/dev/null; FLICK_WATCHING=0 }

_flick_disconnect() {
  [[ -n $FLICK_FD ]] || return 0
  _flick_unwatch
  exec {FLICK_FD}>&-
  FLICK_FD=
}

# zle -F handler: the background process wrote while the shell was idle.
_flick_on_data() {
  local line got=0
  while read -t 0 -r -u $1 line; do got=1; done
  if (( ! got )) || [[ -n $2 ]]; then  # nothing readable: the connection closed
    _flick_disconnect
    return
  fi
  _flick_unwatch
  (( $#FLICK_INSERT || FLICK_OPEN )) || [[ -n $LBUFFER ]] || return
  FLICK_LAST=
  _flick_query && _flick_render || _flick_clear
  zle -R
}

# Words of the command under the cursor, with the command's alias expanded.
# The last element is the word being typed ('' right after a space).
_flick_words() {
  local -a words=(${(z)LBUFFER})
  [[ $LBUFFER == *[[:space:]] || -z $LBUFFER ]] && words+=('')
  local i
  for (( i = $#words; i > 0; i-- )); do
    [[ ${words[i]} == (\;|\||\|\||\&|\&\&|\|\&|\(|\)|\{|\}|do|then|else) ]] && break
  done
  words=("${(@)words[i+1,-1]}")
  (( $#words > 1 )) && [[ -n ${aliases[${words[1]}]} ]] && words=(${(z)aliases[${words[1]}]} "${(@)words[2,-1]}")
  reply=("${(@Q)words}")
}

# Escapes \, tab and newline so a value fits in one protocol field. Result in REPLY.
_flick_escape() { REPLY=${${${1//\\/\\\\}//$'\t'/\\t}//$'\n'/\\n} }

# Sends one request line; reconnects once if the background process went away.
_flick_send() {
  _flick_connect || return 1
  print -r -u $FLICK_FD -- $1 2>/dev/null && return 0
  _flick_disconnect
  _flick_connect && print -r -u $FLICK_FD -- $1 2>/dev/null
}

# Asks the background process for Suggestions for LBUFFER.
_flick_query() {
  FLICK_KIND=() FLICK_INSERT=() FLICK_DESC=() FLICK_SEL=1 FLICK_LAST=$LBUFFER
  local -a reply; _flick_words
  FLICK_WORD=${reply[-1]}
  # One request per line: tabs and newlines inside words (e.g. from $'\n') must not leak.
  # Quoted with (@) so the empty word after a trailing space survives.
  reply=("${(@)reply//[$'\t\n']/ }")
  local REPLY dir line
  _flick_escape $PWD; dir=$REPLY
  _flick_escape $LBUFFER
  _flick_send "c"$'\t'$dir$'\t'$REPLY$'\t'${(pj:\t:)reply} || return 1
  while IFS= read -r -u $FLICK_FD line; do
    [[ $line == '!' ]] && continue  # a refresh notice, not part of this answer
    [[ $line == '~' ]] && { _flick_watch; continue }
    [[ -z $line ]] && break
    FLICK_KIND+=${line[1]}
    line=${line[3,-1]}
    FLICK_INSERT+=${line%%$'\t'*} FLICK_DESC+=${line#*$'\t'}
  done
}

_flick_clear() {
  FLICK_OPEN=0
  POSTDISPLAY=
  region_highlight=(${region_highlight:#*memo=flick})
}

# Draws Ghost text and the Menu from the current Suggestions.
_flick_render() {
  _flick_clear
  (( $#FLICK_INSERT )) && (( CURSOR == $#BUFFER )) || return
  local pick=${FLICK_INSERT[FLICK_SEL]} ghost= typed=$FLICK_WORD
  [[ ${FLICK_KIND[FLICK_SEL]} == l ]] && typed=$LBUFFER
  (( FLICK_NO_GHOST )) || [[ $pick != ${typed}* ]] || ghost=${pick#$typed}
  local s=$#BUFFER
  POSTDISPLAY=$ghost
  region_highlight+=("$s $(( s + $#ghost )) fg=8,memo=flick")
  # Short windows and multi-line commands get Ghost text only.
  (( LINES >= 10 )) && [[ $BUFFER != *$'\n'* ]] || return

  local -i n=$#FLICK_INSERT rows=$(( FLICK_ROWS < $#FLICK_INSERT ? FLICK_ROWS : $#FLICK_INSERT ))
  local -i top=$(( FLICK_SEL > rows ? FLICK_SEL - rows + 1 : 1 ))  # scroll to keep the selection visible
  local -i namew=0 i
  for (( i = top; i < top + rows; i++ )); do (( namew = $#FLICK_INSERT[i] > namew ? $#FLICK_INSERT[i] : namew )); done
  (( namew = namew > 32 ? 32 : namew ))
  local row more= start
  (( n > rows )) && more=" ($FLICK_SEL/$n)"
  for (( i = top; i < top + rows; i++ )); do
    row="${(r:namew:)FLICK_INSERT[i]}  ${FLICK_DESC[i]}"
    (( i == top )) && row+=$more
    row=${row[1,COLUMNS-1]}
    start=$(( s + $#POSTDISPLAY + 1 ))
    POSTDISPLAY+=$'\n'$row
    if (( i == FLICK_SEL )); then
      region_highlight+=("$start $(( start + $#row )) standout,memo=flick")
    else
      region_highlight+=("$start $(( start + namew )) fg=6,memo=flick")
      region_highlight+=("$(( start + namew )) $(( start + $#row )) fg=8,memo=flick")
    fi
  done
  FLICK_OPEN=1
}

# Replaces the word being typed (or the whole line, for History) with the selected Suggestion.
# $1 = suffix to add after a word.
_flick_take() {
  local pick=${FLICK_INSERT[FLICK_SEL]}
  if [[ ${FLICK_KIND[FLICK_SEL]} == l ]]; then
    LBUFFER=$pick
    return
  fi
  LBUFFER=${LBUFFER%$FLICK_WORD}$pick
  [[ -n $1 && $pick != *[/=:] ]] && LBUFFER+=$1
}

_flick_move() {  # $1 = +1 or -1, wraps around
  FLICK_SEL=$(( (FLICK_SEL - 1 + $1 + $#FLICK_INSERT) % $#FLICK_INSERT + 1 ))
  _flick_render
}

# Runs before every redraw, so any edit (typing, paste, kill, undo) refreshes the Menu.
_flick_pre_redraw() {
  [[ $LBUFFER == $FLICK_LAST ]] && return
  if [[ $FLICK_TRIGGER == typing && $LASTWIDGET == (*self-insert*|*delete-char*|*kill*|*paste*|undo|redo|_flick_*) ]]; then
    (( FLICK_NO_GHOST )) || (( ! $+functions[_zsh_autosuggest_start] )) || {
      FLICK_NO_GHOST=1; zle -M "flick: zsh-autosuggestions is loaded; flick won't draw Ghost text. Remove it from your plugins."
    }
    _flick_query && _flick_render || _flick_clear
  else
    FLICK_INSERT=() FLICK_LAST=$LBUFFER
    _flick_clear
  fi
}

_flick_line_finish() { _flick_clear; FLICK_INSERT=() FLICK_LAST= }

# History: remember the command when it starts, record it with its exit code when it ends.
_flick_preexec() { FLICK_CMD=$1 }
_flick_precmd() {
  local exit=$?
  [[ -n $FLICK_CMD ]] || return 0
  local REPLY dir
  _flick_escape $PWD; dir=$REPLY
  _flick_escape $FLICK_CMD
  _flick_send "r"$'\t'$EPOCHSECONDS$'\t'$exit$'\t'$dir$'\t'$REPLY
  FLICK_CMD=
  return 0
}

_flick_down() { (( FLICK_OPEN )) && _flick_move +1 || zle .down-line-or-history }
_flick_up()   { (( FLICK_OPEN )) && _flick_move -1 || zle .up-line-or-history }
_flick_esc()  { _flick_clear }
# Tab: take the selected Suggestion. With the Menu closed, open it; with nothing to suggest,
# hand over to Native completion.
_flick_tab() {
  if (( $#FLICK_INSERT )) && [[ -n $POSTDISPLAY ]]; then  # Menu or Ghost text is showing
    _flick_take ' '; FLICK_LAST=; _flick_query && _flick_render
  elif _flick_query && (( $#FLICK_INSERT )); then
    _flick_render
  else
    _flick_clear
    zle $FLICK_NATIVE_TAB
  fi
}
# Right arrow: take the Ghost text. Otherwise move right as usual.
_flick_right() {
  if (( $#FLICK_INSERT && ! FLICK_NO_GHOST )) && [[ -n $POSTDISPLAY ]]; then
    _flick_take; FLICK_LAST=; _flick_query && _flick_render
  else
    zle .forward-char
  fi
}

add-zsh-hook preexec _flick_preexec
add-zsh-hook precmd _flick_precmd
add-zle-hook-widget line-pre-redraw _flick_pre_redraw
add-zle-hook-widget line-finish _flick_line_finish
for w in _flick_down _flick_up _flick_esc _flick_tab _flick_right _flick_on_data; do zle -N $w; done
bindkey '^[[B' _flick_down  '^[OB' _flick_down
bindkey '^[[A' _flick_up    '^[OA' _flick_up
bindkey '^[[C' _flick_right '^[OC' _flick_right
bindkey '^I'   _flick_tab
bindkey '^['   _flick_esc
unset w
