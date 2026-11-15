# timeit.py <label> <cwd> <stdout file> -- <cmd...>: runs cmd, prints wall, user, sys, maxrss.
import os, sys, time
label, cwd, out = sys.argv[1:4]
cmd = sys.argv[5:]
t0 = time.monotonic()
pid = os.fork()
if pid == 0:
    os.chdir(cwd)
    fd = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    os.dup2(fd, 1)
    os.execv(cmd[0], cmd)
_, status, ru = os.wait4(pid, 0)
wall = time.monotonic() - t0
print("%s wall=%.3f user=%.3f sys=%.3f cpu=%.3f maxrss=%dkB status=%d" % (label, wall, ru.ru_utime, ru.ru_stime, ru.ru_utime + ru.ru_stime, ru.ru_maxrss, os.waitstatus_to_exitcode(status)))
