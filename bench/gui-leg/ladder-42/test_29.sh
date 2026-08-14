#!/bin/sh
# Feature 29: export command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_29): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set b 2 && sh ./minidb set a 1 || fail set
got=$(sh ./minidb export)
want=$(printf 'a\t1\nb\t2')
[ "$got" = "$want" ] || fail "export should print sorted TSV, got: $got"
exit 0
