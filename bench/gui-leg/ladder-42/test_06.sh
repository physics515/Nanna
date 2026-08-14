#!/bin/sh
# Feature 6: independent keys
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_06): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
[ "$(sh ./minidb get a)" = "1" ] || fail "a should be 1"
[ "$(sh ./minidb get b)" = "2" ] || fail "b should be 2"
exit 0
