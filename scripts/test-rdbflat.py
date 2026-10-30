#!/usr/bin/env python3
"""Integration check; run inside robigo-oracle:4.5.0 after scripts/build.sh."""
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BINARY = Path(os.environ.get('ROBIGO_BINARY', ROOT / '.robigo/bin/robigo'))
ENV = dict(os.environ, R_DEFAULT_PACKAGES='NULL', ROBIGO_RDBFLAT='', ROBIGO_RDBLOG='')


def run(command, env=ENV):
    return subprocess.run(command, env=env, capture_output=True, check=True).stdout


def transcript(command, env):
    result = subprocess.run(command, env=env, capture_output=True)
    return result.stdout, result.stderr, result.returncode


with tempfile.TemporaryDirectory() as tmp:
    cache = Path(tmp) / 'cache'
    script = ROOT / 'scripts/build-rdbflat-index.R'
    oracle = run(['Rscript', '--vanilla', str(script)])
    actual = run([str(BINARY), '--vanilla', str(script)])
    assert actual == oracle, 'index enumeration differs from GNU R readRDS'
    log = Path(tmp) / 'index.tsv'
    log.write_bytes(actual)
    run(['python3', str(ROOT / 'scripts/build-rdbflat.py'), str(log), str(cache)])
    indexed = {(p, int(o), int(n)) for p, o, n, _ in
               (line.split('\t') for line in actual.decode().splitlines())}
    cached = set()
    for flat in cache.glob('*.rdb.flat'):
        image = flat.read_bytes()
        plen = struct.unpack_from('<Q', image, 24)[0]
        source = image[32:32 + plen].decode()
        count = struct.unpack_from('<Q', image, 32 + plen)[0]
        for j in range(count):
            off, length, _, _ = struct.unpack_from('<QIIQ', image, 40 + plen + j * 24)
            cached.add((source, off, length))
    assert indexed == cached, 'flat cache does not cover every indexed key'
    broken = Path(tmp) / 'broken'
    shutil.copytree(cache, broken)
    for flat in broken.glob('*.flat'):
        flat.write_bytes(b'RFLAT01\n')
    for packages in ('NULL', 'utils,stats', 'methods,utils,stats,graphics,grDevices,datasets'):
        expr = 'print(search()); print(sum(1:10)); '
        if 'stats' in packages:
            expr += 'print(coef(lm(c(1,3,2,5) ~ c(2,4,6,8)))); print(pnorm(c(-2,0,2)))'
        env = dict(ENV, R_DEFAULT_PACKAGES=packages)
        expected = transcript(['Rscript', '--vanilla', '-e', expr], env)
        assert expected[2] == 0, expected
        for compile, jit in (('0','0'), ('1','0'), ('1','1')):
            for directory in ('', str(cache), str(broken)):
                candidate_env = dict(env, ROBIGO_RDBFLAT=directory,
                                     ROBIGO_COMPILE=compile, ROBIGO_JIT=jit)
                result = transcript([str(BINARY), '--vanilla', '-e', expr], candidate_env)
                assert result == expected, (packages, compile, jit, directory)
    # A malformed key must fail rather than publish a plausible cache.
    log.write_text(next(iter(indexed))[0] + '\t0\t2147483647\t1\n')
    failed = subprocess.run(['python3', str(ROOT / 'scripts/build-rdbflat.py'),
                             str(log), str(Path(tmp) / 'bad')], capture_output=True)
    assert failed.returncode != 0 and not (Path(tmp) / 'bad').exists()
    print(f'{len(indexed)} unique keys match GNU R; 27 startup transcript comparisons pass')
