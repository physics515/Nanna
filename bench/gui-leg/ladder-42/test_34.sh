#!/bin/sh
# Feature 34: delp prefix delete
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_34): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb mset user:a 1 user:b 2 other 3 || fail mset
out=$(sh ./minidb delp user:) || fail "delp should exit 0"
[ "$out" = "2" ] || fail "delp should print 2, got: $out"
[ "$(sh ./minidb count)" = "1" ] || fail "one key should remain"
exit 0
