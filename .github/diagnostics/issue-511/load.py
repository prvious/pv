"""Generates load until killed. Usage: python3 load.py <cpu|storm|suite> <scratch-dir> [checkout]

cpu:   two busy loops per core, no file activity.
storm: one file create/rename/chmod/delete loop per core.
suite: the full workspace test suite, twice at once, over and over (the CI failures' condition).
       Set PV511_TOOLCHAIN to pick a rustup toolchain, such as 1.100.0.
"""

import os
import signal
import subprocess
import sys
from pathlib import Path

mode, scratch = sys.argv[1], Path(sys.argv[2])
scratch.mkdir(parents=True, exist_ok=True)
cpus = os.cpu_count() or 4
CPU = "import hashlib\nb=b'x'*65536\nwhile True: hashlib.sha256(b).digest()"
STORM = (
    "import os,sys\nd=sys.argv[1];os.makedirs(d,exist_ok=True);i=0\n"
    "while True:\n"
    " p=os.path.join(d,f'f{i%200}');t=p+'.tmp'\n"
    " open(t,'w').write('x'*64);os.chmod(t,0o600);os.rename(t,p);os.chmod(p,0o600)\n"
    " if i%3==0: os.unlink(p)\n"
    " i+=1"
)
SUITE = (
    "import subprocess,sys\n"
    "toolchain=sys.argv[3:]\n"
    "while True: subprocess.run(['cargo',*toolchain,'nextest','run','--workspace','--all-features',"
    "'--profile','ci','--retries','0','--no-fail-fast'],cwd=sys.argv[1],"
    "stdout=open(sys.argv[2],'a'),stderr=subprocess.STDOUT)"
)

children = []
if mode == "cpu":
    children = [subprocess.Popen([sys.executable, "-c", CPU]) for _ in range(cpus * 2)]
elif mode == "storm":
    children = [subprocess.Popen([sys.executable, "-c", STORM, str(scratch / f"storm-{i}")]) for i in range(cpus)]
elif mode == "suite":
    # Own process groups, so stopping kills each suite's cargo and test processes and nothing else.
    toolchain = [f"+{os.environ['PV511_TOOLCHAIN']}"] if os.environ.get("PV511_TOOLCHAIN") else []
    children = [subprocess.Popen([sys.executable, "-c", SUITE, sys.argv[3], str(scratch / f"suite-{i}.log"), *toolchain], start_new_session=True) for i in range(2)]


def stop(*_):
    for child in children:
        try:
            os.killpg(child.pid, signal.SIGKILL) if mode == "suite" else child.kill()
        except ProcessLookupError:
            pass
    sys.exit(0)


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
for child in children:
    child.wait()
