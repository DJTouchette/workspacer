#!/usr/bin/env python3
"""Read-only Linux CPU/PSS comparison of running native and Electron clients.

Pass the native PID and Electron MAIN PID. Includes Chromium descendants with
the same executable, but stops at backend processes (hub, claudemon, agents).
PSS apportions shared pages instead of double-counting them as summed RSS does.
This measures current workloads, not equivalent functionality or GPU memory.
"""
import argparse
import json
import os
from pathlib import Path
import statistics
import time


def process(pid):
    root = Path('/proc') / str(pid)
    stat = (root / 'stat').read_text().rsplit(')', 1)[1].split()
    return {
        'pid': pid,
        'parent': int(stat[1]),
        'start': int(stat[19]),
        'ticks': int(stat[11]) + int(stat[12]),
        'exe': os.readlink(root / 'exe'),
    }


def client_processes(root_pid):
    root = process(root_pid)
    candidates = []
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                row = process(int(entry.name))
                if row['exe'] == root['exe']:
                    candidates.append(row)
            except (OSError, ValueError, IndexError):
                continue
    selected = {root_pid: root}
    while True:
        children = {row['pid']: row for row in candidates
                    if row['parent'] in selected and row['pid'] not in selected}
        if not children:
            return selected
        selected.update(children)


def snapshot(root_pid):
    rows = client_processes(root_pid)
    pss = 0
    for pid in rows:
        lines = (Path('/proc') / str(pid) / 'smaps_rollup').read_text().splitlines()
        pss += int(next(line.split()[1] for line in lines if line.startswith('Pss:')))
    return rows, pss / 1024


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-pid', required=True, type=int)
    parser.add_argument('--electron-pid', required=True, type=int)
    parser.add_argument('--seconds', type=float, default=20)
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error('--seconds must be positive')
    roots = {'native': args.native_pid, 'electron': args.electron_pid}
    before = {name: snapshot(pid) for name, pid in roots.items()}
    samples = {name: [value[1]] for name, value in before.items()}
    started = time.monotonic()
    while time.monotonic() - started < args.seconds:
        time.sleep(min(1, max(0, args.seconds - (time.monotonic() - started))))
        after = {name: snapshot(pid) for name, pid in roots.items()}
        for name, value in after.items():
            samples[name].append(value[1])
    elapsed = time.monotonic() - started
    report = {'elapsed_seconds': round(elapsed, 2), 'clients': {}}
    for name in roots:
        first, last = before[name][0], after[name][0]
        stable = {pid: row['start'] for pid, row in first.items()} == {
            pid: row['start'] for pid, row in last.items()}
        ticks = sum(row['ticks'] for row in last.values()) - sum(
            row['ticks'] for row in first.values())
        report['clients'][name] = {
            'pids': sorted(last),
            'mean_pss_mib': round(statistics.mean(samples[name]), 1),
            'peak_pss_mib': round(max(samples[name]), 1),
            'cpu_percent_one_core': round(100 * ticks / os.sysconf('SC_CLK_TCK') / elapsed, 2)
            if stable else None,
            'process_set_stable': stable,
        }
    report['scope'] = (
        'Current workload; 100% CPU = one core. PSS apportions shared RAM; '
        'GPU memory is not measured. Backend processes and dev build tools excluded. '
        'Electron main-process services and any DevTools renderers are included. '
        'Use matching release builds, visibility and conversations for a controlled comparison.'
    )
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
