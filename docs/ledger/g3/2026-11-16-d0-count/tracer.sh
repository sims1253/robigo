#!/bin/bash
# D0c's tracer, installed as bin/R of a copy of the GNU R build tree (the
# real script moved to bin/R.real): each invocation the Makefiles make runs
# GNU R under $D0C_TOOL and leaves a numbered record in $D0C_LOG:
#   NNN.argv NNN.env NNN.stdin   what the Makefile passed
#   NNN.cg                       callgrind's profile   (D0C_TOOL=callgrind)
#   NNN.split NNN.stdout         split.gdb.py's trace  (D0C_TOOL=gdb, D0C_GDB=split.gdb.py)
LOG=${D0C_LOG:?}
mkdir -p "$LOG"
n=$(ls "$LOG" | grep -c '\.argv$')
id=$(printf %03d "$n")
{ echo "cwd=$PWD"; printf 'arg=%s\n' "$@"; } > "$LOG/$id.argv"
env | grep -E '^(R_[A-Z_]*|_R_[A-Z_]*|LC_ALL|TZ)=' | sort > "$LOG/$id.env"
cat > "$LOG/$id.stdin"
case "${D0C_TOOL:-callgrind}" in
  callgrind) exec "$(dirname "$0")/R.real" -d "valgrind --tool=callgrind --callgrind-out-file=$LOG/$id.cg" "$@" < "$LOG/$id.stdin" ;;
  gdb) D0C_SPLIT_OUT=$LOG/$id.split exec "$(dirname "$0")/R.real" -d gdb --debugger-args="-batch -nx -x ${D0C_GDB:?}" "$@" < "$LOG/$id.stdin" > "$LOG/$id.stdout" ;;
  none) exec "$(dirname "$0")/R.real" "$@" < "$LOG/$id.stdin" ;;
esac
