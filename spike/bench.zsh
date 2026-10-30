#!/bin/zsh -f
# Spike benchmark: round-trip latency, in-widget latency while typing,
# startup cost, daemon memory. Run: zsh spike/bench.zsh
emulate -L zsh
zmodload zsh/datetime zsh/zpty
here=${0:A:h}
export TMPDIR=${TMPDIR:-/tmp}
sock=$TMPDIR/flick-$UID.sock

pkill -f "flickd $sock" 2>/dev/null
$here/target/release/flickd $sock &!
for i in {1..50}; do [[ -S $sock ]] && break; sleep 0.02; done

pct() {  # pct <percentile> <values...>
  local p=$1; shift
  local -a s=(${(f)"$(print -l -- $@ | sort -g)"})
  print -- ${s[$(( (${#s} * p + 99) / 100 ))]}
}
stats() { printf '  p50 %.3f ms   p95 %.3f ms   p99 %.3f ms   max %.3f ms\n' \
  $(pct 50 $@) $(pct 95 $@) $(pct 99 $@) $(pct 100 $@) }

print "1. Round-trip shell -> daemon -> shell (2000 queries, no drawing)"
source $here/flick.zsh
_flick_connect
prefixes=(g gi git 'git c' 'git ch' d do doc k ku kub n np npm c ca car l ls x)
times=()
for i in {1..2000}; do
  t0=$EPOCHREALTIME
  _flick_query ${prefixes[$(( i % $#prefixes + 1 ))]}
  times+=$(( (EPOCHREALTIME - t0) * 1000 ))
done
stats $times

print "2. Per keystroke inside ZLE (real interactive zsh, typing 'git checkout main' x20)"
log=$TMPDIR/flick-bench.log; : >| $log
zpty flicktest "FLICK_LOG=$log zsh -f -i"
zpty -w flicktest "source $here/flick.zsh"
sleep 0.3
for r in {1..20}; do
  for c in ${(s::):-git checkout main}; do zpty -w -n flicktest "$c"; sleep 0.01; zpty -r -t flicktest junk; done
  zpty -w -n flicktest $'\x15'   # Ctrl-U: clear line
  sleep 0.05; while zpty -r -t flicktest junk; do :; done
done
sleep 0.3; zpty -d flicktest
times=(${(f)"$(<$log)"})
print "  ($#times keystrokes measured)"; stats $times

print "3. Startup cost of sourcing the plugin (100 shells)"
times=()
for i in {1..100}; do
  t0=$EPOCHREALTIME; zsh -f -c "source $here/flick.zsh"; t1=$EPOCHREALTIME
  t2=$EPOCHREALTIME; zsh -f -c ":"; t3=$EPOCHREALTIME
  times+=$(( ((t1 - t0) - (t3 - t2)) * 1000 ))
done
stats $times

print "4. Daemon memory"
pid=$(pgrep -n -f "flickd $sock")
printf '  RSS %s KB (history entries loaded from %s)\n' "$(ps -o rss= -p $pid | tr -d ' ')" ~/.zsh_history
pkill -f "flickd $sock"
