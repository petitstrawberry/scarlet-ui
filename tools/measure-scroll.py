#!/usr/bin/env python3
"""Run release native wheel benchmarks and retain raw logs plus percentile data.

Work rate is reciprocal measured dispatch/submission time, not physical display
scanout or whole-loop FPS (OS event-pump time is excluded).
Run without other builds/benchmarks; repeat runs to estimate host scheduling noise.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


def read_result(path):
    content = path.read_text()
    result = json.loads(next(line for line in reversed(content.splitlines()) if line.startswith('{')))
    # Older development binaries used an ambiguous label for reciprocal work time.
    if 'submitted_per_s' in result:
        result['work_rate_per_s'] = result.pop('submitted_per_s')
    if 'accepted' in result:
        result['accepted_including_warmup'] = result.pop('accepted')
    warm = content.split('[ScrollBenchmark] warmup_complete', 1)[1]
    for field in ['input_us', 'layout_us', 'prepare_us', 'lower_us', 'encode_us', 'backend_us', 'retained_draws', 'frame_vertex_bytes']:
        values = [int(value) for value in re.findall(r'\b' + field + r'=(\d+)', warm)]
        if values:
            result[field] = {'median': percentile(values, .5), 'p95': percentile(values, .95), 'p99': percentile(values, .99), 'max': max(values)}
    result['log'] = str(path)
    result['retained'] = int(re.search(r'retained-(\d)', path.name).group(1))
    result['repeat'] = int(re.search(r'run-(\d+)', path.name).group(1))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/examples/scroll-benchmark')
    parser.add_argument('--output', type=Path, default=ROOT / 'artifacts/scroll-retained')
    parser.add_argument('--seconds', type=float, default=10.)
    parser.add_argument('--delta', type=int, default=128)
    parser.add_argument('--repeats', type=int, default=1)
    parser.add_argument('--case', choices=['all', 'plain', 'artwork', 'complex'], default='all')
    args = parser.parse_args()
    if args.seconds <= 0 or args.repeats < 1 or args.delta <= 0:
        parser.error('seconds, repeats, and delta must be positive')
    args.output.mkdir(parents=True, exist_ok=True)
    results = []
    cases = ['plain', 'artwork', 'complex'] if args.case == 'all' else [args.case]
    for repeat in range(args.repeats):
        for case in cases:
            for update in [0, 60]:
                # Alternate order across repeats to reduce order-dependent bias.
                for enabled in ([0, 1] if repeat % 2 == 0 else [1, 0]):
                    path = args.output / f'{case}-updates-{update}-retained-{enabled}-run-{repeat}.log'
                    env = dict(os.environ, SCARLET_UI_FRAME_LOG='1', SCARLET_UI_RETAINED_PAINT=str(enabled), SCARLET_UI_WINIT_RENDERER='sgfx')
                    with path.open('w') as stream:
                        subprocess.run([str(args.binary.resolve()), case, str(args.seconds), str(args.delta), str(update)], cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
                    result = read_result(path)
                    results.append(result)
                    print(f'{path.name}: p95={result["p95_ms"]:.3f} ms, p99={result["p99_ms"]:.3f} ms', flush=True)
                    report = {'scope': 'native SGFX/wgpu; synchronous wheel dispatch through presentation acceptance; work rate excludes OS event pumping; physical scanout and platform event-queue latency are not measured', 'command': vars(args) | {'binary': str(args.binary), 'output': str(args.output)}, 'results': results}
                    (args.output / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')


if __name__ == '__main__':
    main()
