#!/usr/bin/env python3
"""Alternate an existing release and the current scalar/SIMD CLI; never build mid-run."""
import argparse
import hashlib
import json
import platform
import subprocess
import tempfile
import time
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--baseline', required=True, type=Path)
p.add_argument('--binary', default=Path('target/release/silicon'), type=Path)
p.add_argument('--frames', default=20, type=int)
p.add_argument('--output', default=Path('output/comparison.json'), type=Path)
a = p.parse_args()
if a.frames < 1:
    p.error('--frames must be positive')
binary = a.binary.resolve()
baseline = a.baseline.resolve()
record = {
    'timestamp': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
    'platform': platform.platform(),
    'cpu': platform.processor(),
    'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
    'worktree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], text=True)),
    'binaries': {
        name: {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
               'info': subprocess.check_output([str(path), 'info'], text=True)}
        for name, path in [('baseline', baseline), ('current', binary)]
    },
    'samples': [],
}
# Two rounds, reversing configuration order to expose drift rather than hiding it.
configurations = [(label, path, backend, workers)
                  for workers in (1, 4)
                  for label, path, backend in [('baseline', baseline, 'simd'),
                                               ('current', binary, 'scalar'),
                                               ('current', binary, 'simd')]]
with tempfile.TemporaryDirectory(prefix='silicon-benchmark-') as temporary:
    report = Path(temporary) / 'frames.json'
    for round_number, configurations_in_round in enumerate((configurations, configurations[::-1])):
        for label, path, backend, workers in configurations_in_round:
            command = [str(path), 'benchmark', 'spirv_showcase', '--frames', str(a.frames),
                       '--backend', backend, '--threads', str(workers)]
            if label == 'current':
                command += ['--report', str(report)]
            run = subprocess.run(command, check=True, text=True, capture_output=True)
            print(f'{label} / {backend} / {workers} workers / round {round_number + 1}', flush=True)
            print(run.stdout, end='', flush=True)
            sample = {'binary': label, 'backend': backend, 'workers': workers,
                      'round': round_number + 1, 'command': command, 'stdout': run.stdout}
            if label == 'current':
                sample['report'] = json.loads(report.read_text())
            record['samples'].append(sample)
            a.output.parent.mkdir(parents=True, exist_ok=True)
            a.output.write_text(json.dumps(record, indent=2) + '\n')
