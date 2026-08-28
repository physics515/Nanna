#!/bin/sh
# Feature 22: mset command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_22): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb mset a 1 b 2 c 3 || fail "mset should exit 0"
[ "$(sh ./minidb get a)" = "1" ] || fail a
[ "$(sh ./minidb get b)" = "2" ] || fail b
[ "$(sh ./minidb get c)" = "3" ] || fail c
exit 0
