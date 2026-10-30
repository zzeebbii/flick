# flick: zsh side. Source this from .zshrc after oh-my-zsh/compinit, before zsh-syntax-highlighting.
# Must print nothing at startup (p10k instant prompt).
zmodload zsh/net/socket zsh/zselect zsh/datetime 2>/dev/null || return
autoload -Uz add-zle-hook-widget add-zsh-hook

typeset -g FLICK_SOCK=${FLICK_SOCK:-${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/flick-$UID.sock}
(( $#FLICK_SOCK < 100 )) || FLICK_SOCK=/tmp/flick-$UID.sock  # macOS caps socket paths at ~104 bytes
typeset -g FLICK_BIN=${0:A:h}/target/release/flick
[[ -x $FLICK_BIN ]] || FLICK_BIN=${commands[flick]:-}

# Settings from config.toml, via `flick config zsh`. Cached: regenerated only when the config,
# a theme or the binary is newer than the cache, so startup normally just sources one file.
typeset -g FLICK_TRIGGER=typing FLICK_NATIVE=1 FLICK_CONFIG_ERROR=  # Trigger mode: typing | tab
typeset -gi FLICK_ROWS=8
typeset -g FLICK_ALIGN=left  # left | cursor: where the Menu starts
typeset -g FLICK_STYLE_GHOST=fg=8 FLICK_STYLE_NAME=fg=6 FLICK_STYLE_DESC=fg=8 FLICK_STYLE_SELECTED=standout FLICK_STYLE_HISTORY=fg=5
typeset -g FLICK_STYLE_OPTION=fg=3 FLICK_STYLE_FILE= FLICK_STYLE_FOLDER=fg=4 FLICK_STYLE_VALUE=fg=2 FLICK_STYLE_MATCH=bold FLICK_STYLE_PANEL=
typeset -g FLICK_ICON_HISTORY= FLICK_ICON_COMMAND= FLICK_ICON_OPTION= FLICK_ICON_FILE= FLICK_ICON_FOLDER=
typeset -g FLICK_KEY_ACCEPT='^I' FLICK_KEY_ACCEPT_GHOST='^[[C' FLICK_KEY_UP='^[[A' FLICK_KEY_DOWN='^[[B' FLICK_KEY_CLOSE='^['
() {
  emulate -L zsh
  local cfg=${FLICK_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/flick/config.toml}
  local cache=${XDG_CACHE_HOME:-$HOME/.cache}/flick/config.zsh first=
  local -a theme=(${cfg:h}/themes/*.toml(N.om[1]))  # newest theme file
  [[ -f $cache ]] && read -r first < $cache
  if [[ -n $FLICK_BIN ]] && { [[ $first != "# for $cfg" || $FLICK_BIN -nt $cache || $cfg -nt $cache ]] ||
       [[ -n $theme[1] && $theme[1] -nt $cache ]] }; then
    mkdir -p ${cache:h} 2>/dev/null
    { print -r -- "# for $cfg"; FLICK_CONFIG=$cfg $FLICK_BIN config zsh } >| $cache 2>/dev/null
  fi
  [[ -f $cache ]] && source $cache
}

typeset -g FLICK_FD=
typeset -ga FLICK_KIND FLICK_INSERT FLICK_DESC  # current Suggestions; kind w = word, l = whole line
typeset -g FLICK_WORD=               # the word they replace
typeset -gi FLICK_SEL=1              # selected Suggestion
typeset -gi FLICK_OPEN=0             # Menu is showing
typeset -g FLICK_LAST=               # LBUFFER the Suggestions were made for
typeset -g FLICK_LAST_BUFFER=        # BUFFER at the last redraw, to tell edits from cursor moves
typeset -gi FLICK_NO_GHOST=0         # zsh-autosuggestions owns Ghost text
typeset -g FLICK_CMD=                # command line now running, for History
typeset -gi FLICK_WATCHING=0         # zle -F handler installed on FLICK_FD
typeset -gi FLICK_RETRY_AT=0         # after a failed start, don't try again before this time
typeset -gi FLICK_ORIGIN=0           # screen column (1-based) where typing starts; 0 = unknown
typeset -g FLICK_DSR=auto            # auto | on | off: does the terminal answer position queries?
# Whatever Tab did before flick (expand-or-complete, fzf-tab, ...): Native completion, used
# when flick has nothing to suggest.
typeset -g FLICK_NATIVE_TAB=${${(z)"$(bindkey $FLICK_KEY_ACCEPT)"}[2]:-expand-or-complete}  # `"^I" widget`
[[ $FLICK_NATIVE_TAB == (_flick_tab|undefined-key) ]] && FLICK_NATIVE_TAB=expand-or-complete  # sourced twice

_flick_connect() {
  [[ -n $FLICK_FD ]] && return 0
  if ! zsocket $FLICK_SOCK 2>/dev/null; then
    [[ -n $FLICK_BIN ]] || return 1
    # A start that failed (broken binary, unusable socket path) is not retried on every
    # keystroke: each attempt waits up to 300 ms.
    (( EPOCHSECONDS >= FLICK_RETRY_AT )) || return 1
    FLICK_HISTFILE=${FLICK_HISTFILE:-$HISTFILE} $FLICK_BIN serve $FLICK_SOCK &>/dev/null &!
    # Only the first keystroke after flick starts pays this (~100 ms with a large History).
    local i
    for i in {1..30}; do
      zselect -t 1  # 10 ms, without forking sleep
      zsocket $FLICK_SOCK 2>/dev/null && break
    done || { FLICK_RETRY_AT=$(( EPOCHSECONDS + 30 )); return 1 }
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
  # The command under the cursor starts after the last separator. `do`, `then` and `else` only
  # count where a command could start (`for x in a; do git ch`), not as words (`cd do`).
  local i
  for (( i = $#words - 1; i > 0; i-- )); do
    [[ ${words[i]} == (\;|\||\|\||\&|\&\&|\|\&|\(|\)|\{|\}) ]] && break
    [[ ${words[i]} == (do|then|else) ]] &&
      { (( i == 1 )) || [[ ${words[i-1]} == (\;|\||\|\||\&|\&\&|\|\&|\(|\)|\{|\}|$'\n') ]] } && break
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
  FLICK_KIND=() FLICK_INSERT=() FLICK_DESC=() FLICK_SEL=1 FLICK_LAST=$LBUFFER FLICK_LAST_BUFFER=$BUFFER
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

# Positions (0-based) in $1 of the letters of $2, for the match style: a prefix in any case,
# else the first in-order match (fuzzy). Result in reply; empty if they don't match.
_flick_matched() {
  reply=()
  local c=${(L)1} t=${(L)2}
  [[ -n $t ]] || return 0
  if [[ ${c[1,$#t]} == "$t" ]]; then
    reply=({0..$(( $#t - 1 ))})
    return 0
  fi
  local -i i=1 j=1
  while (( i <= $#c && j <= $#t )); do
    [[ ${c[i]} == "${t[j]}" ]] && { reply+=$(( i - 1 )); (( j++ )) }
    (( i++ ))
  done
  (( j > $#t )) || reply=()
}

# Adds one highlight: $1 start, $2 end, then style parts joined with commas (empty ones skipped).
# Later parts win where they set the same attribute, so the row style goes first.
_flick_hl() {
  (( $2 > $1 )) || return 0
  local spec=${(j:,:)${(@)argv[3,-1]:#}}
  region_highlight+=("$1 $2 ${spec:-none},memo=flick")
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
  region_highlight+=("$s $(( s + $#ghost )) $FLICK_STYLE_GHOST,memo=flick")
  # Short windows and multi-line commands get Ghost text only.
  (( LINES >= 10 )) && [[ $BUFFER != *$'\n'* ]] || return

  local -i n=$#FLICK_INSERT rows=$(( FLICK_ROWS < $#FLICK_INSERT ? FLICK_ROWS : $#FLICK_INSERT ))
  local -i top=$(( FLICK_SEL > rows ? FLICK_SEL - rows + 1 : 1 ))  # scroll to keep the selection visible
  local -i namew=0 i
  for (( i = top; i < top + rows; i++ )); do (( namew = $#FLICK_INSERT[i] > namew ? $#FLICK_INSERT[i] : namew )); done
  (( namew = namew > 32 ? 32 : namew ))
  local more= icon kind
  local -a lines name_styles
  local -i iconw=0 widest=0
  [[ -n $FLICK_ICON_HISTORY$FLICK_ICON_COMMAND$FLICK_ICON_OPTION$FLICK_ICON_FILE$FLICK_ICON_FOLDER ]] && iconw=2
  (( n > rows )) && more=" ($FLICK_SEL/$n)"
  for (( i = top; i < top + rows; i++ )); do
    kind=${FLICK_KIND[i]}
    case $kind in
      l) icon=$FLICK_ICON_HISTORY name_styles+=$FLICK_STYLE_HISTORY ;;
      o) icon=$FLICK_ICON_OPTION  name_styles+=$FLICK_STYLE_OPTION ;;
      d) icon=$FLICK_ICON_FOLDER  name_styles+=$FLICK_STYLE_FOLDER ;;
      f) icon=$FLICK_ICON_FILE    name_styles+=$FLICK_STYLE_FILE ;;
      v) icon=$FLICK_ICON_COMMAND name_styles+=$FLICK_STYLE_VALUE ;;
      *) icon=$FLICK_ICON_COMMAND name_styles+=$FLICK_STYLE_NAME ;;
    esac
    [[ -n $name_styles[-1] ]] || name_styles[-1]=$FLICK_STYLE_NAME
    (( iconw )) && icon="${(r:1:)icon} " || icon=
    lines+="$icon${(r:namew:)FLICK_INSERT[i]}  ${FLICK_DESC[i]}"
    (( i == top )) && lines[-1]+=$more
    (( widest = $#lines[-1] > widest ? $#lines[-1] : widest ))
  done

  # align = cursor: start under the word being typed. Near the right edge, shift left to keep
  # at least 40 columns (or the widest row, if narrower); longer descriptions are cut off.
  local -i indent=0 width=$(( COLUMNS - 1 ))
  if [[ $FLICK_ALIGN == cursor ]] && (( FLICK_ORIGIN )); then
    local before=${LBUFFER%$FLICK_WORD}
    local -i need=$(( widest < 40 ? widest : 40 ))
    (( indent = (FLICK_ORIGIN - 1 + ${(m)#before}) % COLUMNS ))
    (( indent = indent < width - need ? indent : width - need ))
    (( indent = indent > 0 ? indent : 0 ))
  fi

  # Every row is padded to the same width, so a panel background reads as one block. Each row
  # is drawn as separate, non-overlapping parts, each with its full style:
  # icon and name, the typed part of the name (match), the rest of the name, the description.
  local -i pw=$(( widest < width - indent ? widest : width - indent )) start namestart descstart p run hit
  local row pad=${(l:indent:)} base name_style
  for (( i = 1; i <= $#lines; i++ )); do
    row=$lines[i]
    if (( $#row > pw )); then row="${row[1,pw-1]}…"; else row=${(r:pw:)row}; fi
    start=$(( s + $#POSTDISPLAY + 1 + indent ))
    POSTDISPLAY+=$'\n'$pad$row

    typed=$FLICK_WORD
    [[ ${FLICK_KIND[top+i-1]} == l ]] && typed=$LBUFFER
    _flick_matched ${FLICK_INSERT[top+i-1]} $typed
    if (( top + i - 1 == FLICK_SEL )); then
      base=$FLICK_STYLE_SELECTED name_style=
    else
      base=$FLICK_STYLE_PANEL name_style=$name_styles[i]
    fi
    namestart=$(( start + iconw )) descstart=$(( start + iconw + namew ))
    (( descstart = descstart < start + pw ? descstart : start + pw ))
    _flick_hl $start $namestart $base $name_style
    # The name, in runs: matched letters get the match style on top of the row's.
    p=0
    while (( namestart + p < descstart )); do
      hit=$(( ${reply[(Ie)$p]} > 0 ))
      for (( run = p + 1; namestart + run < descstart && $(( ${reply[(Ie)$run]} > 0 )) == hit; run++ )); do :; done
      if (( hit )); then
        _flick_hl $(( namestart + p )) $(( namestart + run )) $base $name_style $FLICK_STYLE_MATCH
      else
        _flick_hl $(( namestart + p )) $(( namestart + run )) $base $name_style
      fi
      p=$run
    done
    if (( top + i - 1 == FLICK_SEL )); then
      _flick_hl $descstart $(( start + pw )) $base
    else
      _flick_hl $descstart $(( start + pw )) $base $FLICK_STYLE_DESC
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

# Runs before every redraw. Any edit refreshes the Menu, whatever widget made it (self-insert,
# magic-space, paste, autopair, ...); cursor moves and recalled History lines close it.
_flick_pre_redraw() {
  [[ $LBUFFER == "$FLICK_LAST" && $BUFFER == "$FLICK_LAST_BUFFER" ]] && return
  [[ -z $FLICK_CONFIG_ERROR ]] || { zle -M "flick: $FLICK_CONFIG_ERROR (using defaults)"; FLICK_CONFIG_ERROR= }
  local edited=0
  [[ $BUFFER != "$FLICK_LAST_BUFFER" ]] && edited=1
  FLICK_LAST_BUFFER=$BUFFER
  if [[ $FLICK_TRIGGER == typing ]] && (( edited )) &&
     [[ $LASTWIDGET != (*history*|*search*|accept-*|push-*|get-line|fzf-*-widget) ]]; then
    (( FLICK_NO_GHOST )) || (( ! $+functions[_zsh_autosuggest_start] )) || {
      FLICK_NO_GHOST=1; zle -M "flick: zsh-autosuggestions is loaded; flick won't draw Ghost text. Remove it from your plugins."
    }
    _flick_query && _flick_render || _flick_clear
  else
    FLICK_INSERT=() FLICK_LAST=$LBUFFER
    _flick_clear
  fi
}

# align = cursor: ask the terminal where typing starts (ESC[6n, answered with ESC[row;colR),
# once per prompt. Keys typed meanwhile arrive mixed with the answer; they are handed back
# to the line editor, in order. The answer ends the wait at once; the 0.5 s limit is generous
# because an answer arriving after it (slow SSH) would be typed into the line as junk.
# A terminal that never answers is not asked again.
_flick_line_init() {
  emulate -L zsh
  FLICK_ORIGIN=0
  [[ $FLICK_ALIGN == cursor && $FLICK_DSR != off ]] || return 0
  local c buf= rest
  print -n $'\e[6n' > /dev/tty
  while IFS= read -rs -k 1 -t 0.5 c < /dev/tty; do
    buf+=$c
    [[ $buf =~ $'\e\\[([0-9]+);([0-9]+)R' ]] && break
  done
  if [[ $buf =~ $'\e\\[([0-9]+);([0-9]+)R' ]]; then
    FLICK_ORIGIN=$match[2] FLICK_DSR=on
    rest=${buf/"$MATCH"}
  else
    rest=$buf
    [[ $FLICK_DSR == auto ]] && FLICK_DSR=off  # never answered, even with keys typed meanwhile
  fi
  [[ -n $rest ]] && zle -U -- "$rest"
  return 0
}

_flick_line_finish() { _flick_clear; FLICK_INSERT=() FLICK_LAST= FLICK_LAST_BUFFER= }

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
    (( FLICK_NATIVE )) && zle $FLICK_NATIVE_TAB
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
add-zle-hook-widget line-init _flick_line_init
add-zle-hook-widget line-finish _flick_line_finish
for w in _flick_down _flick_up _flick_esc _flick_tab _flick_right _flick_on_data; do zle -N $w; done
() {
  local key widget
  for key widget in $FLICK_KEY_DOWN _flick_down $FLICK_KEY_UP _flick_up $FLICK_KEY_ACCEPT_GHOST _flick_right \
                    $FLICK_KEY_ACCEPT _flick_tab $FLICK_KEY_CLOSE _flick_esc; do
    bindkey $key $widget
    # Arrow keys send ^[O… instead of ^[[… in the terminal's "application" mode.
    [[ $key == '^[['[ABCD] ]] && bindkey "^[O${key[-1]}" $widget
  done
}
unset w
