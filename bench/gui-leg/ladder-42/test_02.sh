#!/bin/sh
# Feature 2: help command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_02): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb help >out.txt 2>&1 || fail "help should exit 0"
grep -qi minidb out.txt || fail "help should mention minidb"
grep -q set out.txt || fail "help should mention set"
rm -f out.txt
exit 0
