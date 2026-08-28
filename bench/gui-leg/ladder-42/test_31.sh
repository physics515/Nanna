#!/bin/sh
# Feature 31: import merges
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_31): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set x 1 && sh ./minidb set z 5 || fail set
printf 'x\t9\ny\t8\n' > imp.tsv
sh ./minidb import imp.tsv || fail import
[ "$(sh ./minidb get x)" = "9" ] || fail "x should be overwritten to 9"
[ "$(sh ./minidb get y)" = "8" ] || fail "y should be added"
[ "$(sh ./minidb get z)" = "5" ] || fail "z should be kept"
rm -f imp.tsv
exit 0
