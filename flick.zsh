# flick: zsh side. Source this from .zshrc after oh-my-zsh/compinit, before zsh-syntax-highlighting.
# Must print nothing at startup (p10k instant prompt).
zmodload zsh/net/socket zsh/zselect 2>/dev/null || return
autoload -Uz add-zle-hook-widget

typeset -g FLICK_SOCK=${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/flick-$UID.sock
typeset -g FLICK_BIN=${0:A:h}/target/release/flick
[[ -x $FLICK_BIN ]] || FLICK_BIN=${commands[flick]:-}
typeset -g FLICK_TRIGGER=${FLICK_TRIGGER:-typing}  # typing | tab (Trigger mode)
typeset -gi FLICK_ROWS=${FLICK_ROWS:-8}

typeset -g FLICK_FD=
typeset -ga FLICK_INSERT FLICK_DESC  # current Suggestions
typeset -g FLICK_WORD=               # the word they replace
typeset -gi FLICK_SEL=1              # selected Suggestion
typeset -gi FLICK_OPEN=0             # Menu is showing
typeset -g FLICK_LAST=               # LBUFFER the Suggestions were made for
typeset -gi FLICK_NO_GHOST=0         # zsh-autosuggestions owns Ghost text

_flick_connect() {
  [[ -n $FLICK_FD ]] && return 0
  if ! zsocket $FLICK_SOCK 2>/dev/null; then
    [[ -n $FLICK_BIN ]] || return 1
    $FLICK_BIN serve $FLICK_SOCK &>/dev/null &!
    # Only the first keystroke after flick starts pays this; it is up in a few ms.
    local i
    for i in {1..10}; do
      zselect -t 1  # 10 ms, without forking sleep
      zsocket $FLICK_SOCK 2>/dev/null && break
    done || return 1
  fi
  FLICK_FD=$REPLY
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

# Asks the background process for Suggestions for LBUFFER.
_flick_query() {
  FLICK_INSERT=() FLICK_DESC=() FLICK_SEL=1 FLICK_LAST=$LBUFFER
  _flick_connect || return 1
  local -a reply; _flick_words
  FLICK_WORD=${reply[-1]}
  # One request per line: tabs and newlines inside words (e.g. from $'\n') must not leak.
  # Quoted with (@) so the empty word after a trailing space survives.
  reply=("${(@)reply//[$'\t\n']/ }")
  if ! print -r -u $FLICK_FD -- ${(pj:\t:)reply} 2>/dev/null; then
    exec {FLICK_FD}>&-; FLICK_FD=; return 1
  fi
  local line
  while IFS= read -r -u $FLICK_FD line && [[ -n $line ]]; do
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
  local pick=${FLICK_INSERT[FLICK_SEL]} ghost=
  (( FLICK_NO_GHOST )) || [[ $pick != ${FLICK_WORD}* ]] || ghost=${pick#$FLICK_WORD}
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

# Replaces the word being typed with the selected Suggestion. $1 = suffix to add.
_flick_take() {
  local pick=${FLICK_INSERT[FLICK_SEL]}
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

_flick_down() { (( FLICK_OPEN )) && _flick_move +1 || zle .down-line-or-history }
_flick_up()   { (( FLICK_OPEN )) && _flick_move -1 || zle .up-line-or-history }
_flick_esc()  { _flick_clear }
# Tab: take the selected Suggestion. With the Menu closed, open it; with nothing to suggest, complete as usual.
_flick_tab() {
  if (( $#FLICK_INSERT )) && [[ -n $POSTDISPLAY ]]; then  # Menu or Ghost text is showing
    _flick_take ' '; FLICK_LAST=; _flick_query && _flick_render
  elif _flick_query && (( $#FLICK_INSERT )); then
    _flick_render
  else
    zle expand-or-complete
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

add-zle-hook-widget line-pre-redraw _flick_pre_redraw
add-zle-hook-widget line-finish _flick_line_finish
for w in _flick_down _flick_up _flick_esc _flick_tab _flick_right; do zle -N $w; done
bindkey '^[[B' _flick_down  '^[OB' _flick_down
bindkey '^[[A' _flick_up    '^[OA' _flick_up
bindkey '^[[C' _flick_right '^[OC' _flick_right
bindkey '^I'   _flick_tab
bindkey '^['   _flick_esc
unset w
