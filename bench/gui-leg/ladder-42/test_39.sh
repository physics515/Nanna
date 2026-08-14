#!/bin/sh
# Feature 39: namespaced set/get
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_39): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb nset app k v1 || fail nset1
sh ./minidb nset web k v2 || fail nset2
sh ./minidb set k plain || fail set
[ "$(sh ./minidb nget app k)" = "v1" ] || fail "app ns should hold v1"
[ "$(sh ./minidb nget web k)" = "v2" ] || fail "web ns should hold v2"
[ "$(sh ./minidb get k)" = "plain" ] || fail "plain key should be isolated"
exit 0
