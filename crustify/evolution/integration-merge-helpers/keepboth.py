#!/usr/bin/env python3
"""keepboth.py <file>: resolve every conflict hunk by keeping ours, a blank line, then theirs."""
import re, sys
p = sys.argv[1]; t = open(p).read()
pat = re.compile(r'<<<<<<< [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n', re.S)
t = pat.sub(lambda m: m.group(1).rstrip('\n') + '\n\n' + m.group(2), t)
open(p, 'w').write(t)
