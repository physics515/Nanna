#!/bin/sh
# Feature 35: stats command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_35): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k v || fail set
sh ./minidb stats > out.txt || fail "stats should exit 0"
grep -q '^keys=1$' out.txt || fail "stats should print keys=1"
grep -q '^file=' out.txt || fail "stats should print file="
rm -f out.txt
exit 0
