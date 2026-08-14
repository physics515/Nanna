#!/bin/sh
# Feature 28: vgrep command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_28): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb mset a hello b world c hell || fail mset
got=$(sh ./minidb vgrep hell)
want=$(printf 'a\nc')
[ "$got" = "$want" ] || fail "vgrep hell should print a+c, got: $got"
exit 0
