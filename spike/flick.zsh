# Spike: ghost text + inline menu fed by flickd over a unix socket. Throwaway.
zmodload zsh/net/socket zsh/datetime

typeset -g FLICK_SOCK=${TMPDIR:-/tmp}/flick-$UID.sock
typeset -g FLICK_BIN=${0:A:h}/target/release/flickd
typeset -g FLICK_FD=
typeset -g FLICK_LOG=${FLICK_LOG:-}   # set to a file path to log per-keystroke ms

_flick_connect() {
  [[ -n $FLICK_FD ]] && return 0
  if ! zsocket $FLICK_SOCK 2>/dev/null; then
    [[ -x $FLICK_BIN ]] || return 1
    $FLICK_BIN $FLICK_SOCK &>/dev/null &!
    return 1   # daemon is starting; try again on the next keystroke
  fi
  FLICK_FD=$REPLY
}

# Fills $reply with suggestions for $1. On any I/O failure, drop the fd.
_flick_query() {
  reply=()
  _flick_connect || return 1
  local line
  if ! print -r -u $FLICK_FD -- ${1//$'\n'/ } 2>/dev/null; then
    exec {FLICK_FD}>&-; FLICK_FD=; return 1
  fi
  while read -r -u $FLICK_FD line && [[ -n $line ]]; do reply+=$line; done
}

_flick_update() {
  local t0=$EPOCHREALTIME
  POSTDISPLAY= region_highlight=(${region_highlight:#*memo=flick})
  (( CURSOR == $#BUFFER )) && _flick_query $BUFFER || return
  (( $#reply )) || return
  local ghost=${reply[1]#$BUFFER} menu=${(F)reply}
  POSTDISPLAY=$ghost$'\n'$menu
  local s=$#BUFFER
  region_highlight+=("$s $(( s + $#ghost )) fg=8,memo=flick")
  region_highlight+=("$(( s + $#ghost + 1 )) $(( s + $#POSTDISPLAY )) fg=6,memo=flick")
  [[ -n $FLICK_LOG ]] && print -- $(( (EPOCHREALTIME - t0) * 1000 )) >>| $FLICK_LOG
}

_flick_self_insert() { zle .self-insert; _flick_update }
_flick_backward_delete_char() { zle .backward-delete-char; _flick_update }
_flick_accept_ghost() {
  [[ -n $POSTDISPLAY ]] && BUFFER+=${POSTDISPLAY%%$'\n'*} CURSOR=$#BUFFER
  _flick_update
}
_flick_clear() { POSTDISPLAY= region_highlight=(${region_highlight:#*memo=flick}) }
_flick_accept_line() { _flick_clear; zle .accept-line }

zle -N self-insert _flick_self_insert
zle -N backward-delete-char _flick_backward_delete_char
zle -N accept-line _flick_accept_line
zle -N _flick_accept_ghost
bindkey '^[[C' _flick_accept_ghost   # Right arrow
bindkey '^[OC' _flick_accept_ghost
