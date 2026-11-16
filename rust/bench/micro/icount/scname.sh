#!/bin/bash
# translate "nr:count" pairs using the kernel header
sed 's/ /\n/g' | while IFS=: read nr c; do [ -n "$c" ] || { echo "$nr"; continue; }; nm=$(awk -v n=$nr '$1=="#define" && $3==n {sub("__NR_","",$2); print $2; exit}' /usr/include/x86_64-linux-gnu/asm/unistd_64.h); printf "%s=%s " "${nm:-$nr}" "$c"; done; echo
