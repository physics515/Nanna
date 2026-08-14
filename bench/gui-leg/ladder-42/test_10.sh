#!/bin/sh
# Feature 10: count command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_10): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
[ "$(sh ./minidb count)" = "0" ] || fail "empty count should be 0"
sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
[ "$(sh ./minidb count)" = "2" ] || fail "count should be 2"
exit 0
