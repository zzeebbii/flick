# flick: zsh side. Source this from .zshrc after oh-my-zsh/compinit, before zsh-syntax-highlighting.
# Must print nothing at startup (p10k instant prompt).
zmodload zsh/net/socket 2>/dev/null || return

typeset -g FLICK_SOCK=${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/flick-$UID.sock
typeset -g FLICK_BIN=${0:A:h}/target/release/flick
[[ -x $FLICK_BIN ]] || FLICK_BIN=${commands[flick]:-}
typeset -g FLICK_FD=
typeset -ga FLICK_INSERT FLICK_DESC   # current Suggestions
typeset -g FLICK_WORD=                # the word they replace

_flick_connect() {
  [[ -n $FLICK_FD ]] && return 0
  if ! zsocket $FLICK_SOCK 2>/dev/null; then
    [[ -n $FLICK_BIN ]] && $FLICK_BIN serve $FLICK_SOCK &>/dev/null &!
    return 1   # starting; the next keystroke connects
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

_flick_query() {
  FLICK_INSERT=() FLICK_DESC=()
  _flick_connect || return 1
  local -a reply; _flick_words
  FLICK_WORD=${reply[-1]}
  if ! print -r -u $FLICK_FD -- ${(pj:\t:)${reply//$'\t'/ }} 2>/dev/null; then
    exec {FLICK_FD}>&-; FLICK_FD=; return 1
  fi
  local line
  while IFS= read -r -u $FLICK_FD line && [[ -n $line ]]; do
    FLICK_INSERT+=${line%%$'\t'*} FLICK_DESC+=${line#*$'\t'}
  done
}

_flick_clear() {
  POSTDISPLAY=
  region_highlight=(${region_highlight:#*memo=flick})
}

_flick_draw() {
  _flick_clear
  (( CURSOR == $#BUFFER )) || return
  _flick_query || return
  (( $#FLICK_INSERT )) || return
  local ghost= first=${FLICK_INSERT[1]}
  [[ $first == ${FLICK_WORD}* ]] && ghost=${first#$FLICK_WORD}
  local -a rows; local i width=$(( COLUMNS - 1 ))
  for (( i = 1; i <= $#FLICK_INSERT && i <= 8; i++ )); do
    rows+=${${(r:24:)FLICK_INSERT[i]}}"  "${FLICK_DESC[i]}
    rows[-1]=${rows[-1][1,width]}
  done
  POSTDISPLAY=$ghost$'\n'${(F)rows}
  local s=$#BUFFER
  region_highlight+=("$s $(( s + $#ghost )) fg=8,memo=flick")
  region_highlight+=("$(( s + $#ghost + 1 )) $(( s + $#POSTDISPLAY )) fg=6,memo=flick")
}

_flick_self_insert() { zle .self-insert; _flick_draw }
_flick_backward_delete_char() { zle .backward-delete-char; _flick_draw }
_flick_accept_line() { _flick_clear; zle .accept-line }
# Right arrow: take the top Suggestion for the current word.
_flick_accept() {
  if (( $#FLICK_INSERT )) && [[ -n $POSTDISPLAY ]]; then
    LBUFFER=${LBUFFER%$FLICK_WORD}${FLICK_INSERT[1]}
    _flick_draw
  else
    zle .forward-char
  fi
}

zle -N self-insert _flick_self_insert
zle -N backward-delete-char _flick_backward_delete_char
zle -N accept-line _flick_accept_line
zle -N _flick_accept
bindkey '^[[C' _flick_accept
bindkey '^[OC' _flick_accept
