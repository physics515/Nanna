#!/bin/sh
# Feature 1: usage on no args
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_01): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb >out.txt 2>&1
[ $? -eq 2 ] || fail "exit code should be 2"
grep -qi usage out.txt || fail "should print usage"
rm -f out.txt
exit 0
