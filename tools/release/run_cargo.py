#!/usr/bin/env python3
"""Run Cargo with live output and periodic, bounded native compiler activity reports."""
import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time


MAX_PROCESSES = 8


class Cancelled(BaseException):
    def __init__(self, signum):
        self.signum = signum


def cancel(signum, _frame):
    raise Cancelled(signum)


def cpu_seconds(value):
    days, separator, clock = value.partition('-')
    total = int(days) * 86400 if separator else 0
    fields = (clock if separator else days).split(':')
    for index, field in enumerate(reversed(fields)):
        total += float(field) * 60 ** index
    return total


def process_family(rows, root_pid):
    family = {root_pid}
    while True:
        children = {row['pid'] for row in rows if row['ppid'] in family}
        if children <= family:
            return family
        family.update(children)


def native_diagnostics(root_pid):
    """Return only this Cargo tree's process names/PIDs/CPU/RSS, never command lines."""
    try:
        if os.name == 'nt':
            powershell = shutil.which('pwsh') or shutil.which('powershell')
            if not powershell:
                return []
            # Root PID is our integer Popen PID, not user-provided shell text. Process
            # ancestry comes from CIM; CPU seconds and working set come from Get-Process.
            script = r'''
$ErrorActionPreference = 'Stop'
$rows = @(Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId,Name)
$family = [System.Collections.Generic.HashSet[uint32]]::new()
[void]$family.Add(ROOT_PID)
do {
    $changed = $false
    foreach ($row in $rows) {
        if ($family.Contains($row.ParentProcessId) -and $family.Add($row.ProcessId)) { $changed = $true }
    }
} while ($changed)
$result = @($rows | Where-Object { $family.Contains($_.ProcessId) } | Select-Object -First 32 | ForEach-Object {
    $process = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue
    if ($process) {
        [pscustomobject]@{ pid = $_.ProcessId; ppid = $_.ParentProcessId; name = $_.Name; cpu = $process.CPU; rss = $process.WorkingSet64 / 1024 }
    }
})
ConvertTo-Json -InputObject $result -Compress
'''.replace('ROOT_PID', str(root_pid))
            result = subprocess.run(
                [powershell, '-NoProfile', '-NonInteractive', '-Command', script],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                encoding='utf-8', errors='replace', timeout=5, check=False,
            )
            rows = json.loads(result.stdout) if result.returncode == 0 else []
        else:
            result = subprocess.run(
                # macOS clips a non-final command column to its fixed width even
                # with -ww. Keep comm last so the basename survives long paths.
                ['ps', '-ww', '-eo', 'pid=,ppid=,time=,rss=,comm='],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                timeout=5, check=False,
            )
            rows = []
            if result.returncode == 0:
                for line in result.stdout.splitlines():
                    pid, ppid, cpu, rss, name = line.split(None, 4)
                    rows.append(dict(pid=int(pid), ppid=int(ppid), name=name,
                                     cpu=cpu_seconds(cpu), rss=int(rss)))
        family = process_family(rows, root_pid)
        details = []
        for row in rows:
            name = row['name'].replace('\\', '/').rsplit('/', 1)[-1]
            # Include active test executables and build scripts too. Cargo itself
            # waits with zero CPU while those descendants do the actual work.
            if row['pid'] not in family:
                continue
            # A bounded basename prevents large/untrusted process names entering logs.
            name = ''.join(c for c in name[:32] if c.isalnum() or c in '-_.+')
            details.append(f"{name} pid={row['pid']} CPU={float(row['cpu'] or 0):.1f}s RSS={float(row['rss']) / 1024:.0f}MiB")
            if len(details) == MAX_PROCESSES:
                break
        return details
    except (OSError, subprocess.SubprocessError, ValueError, TypeError, KeyError):
        return []


def heartbeat(child, started):
    elapsed = int(time.monotonic() - started)
    print(f'[build] Still running: {elapsed // 60}m {elapsed % 60:02d}s elapsed.', file=sys.stderr, flush=True)
    details = native_diagnostics(child.pid)
    if details:
        print('[build] Process activity: ' + '; '.join(details), file=sys.stderr, flush=True)
    elif child.poll() is None:
        print('[build] Native process activity is unavailable; waiting for Cargo.', file=sys.stderr, flush=True)


def terminate_child(child):
    """Cancel only our process group/tree; never enumerate and kill other Cargo jobs."""
    if child.poll() is not None:
        return
    if os.name == 'nt':
        # Kill the tree while its parent still exists, so descendants cannot escape
        # enumeration after Cargo handles a graceful signal and exits first.
        subprocess.run(['taskkill', '/PID', str(child.pid), '/T', '/F'],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                       timeout=10, check=False)
    else:
        try:
            os.killpg(child.pid, signal.SIGTERM)
            child.wait(timeout=5)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            pass
        # Cargo may have exited before descendants which ignore SIGTERM. The
        # session was created for this child, so finish cancelling that group.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    child.wait(timeout=10)


def run(command, interval):
    options = {'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == 'nt' else {'start_new_session': True}
    # Inherit both streams: neither Cargo output nor diagnostics wait for a pipe reader.
    child = subprocess.Popen(command, **options)
    started = time.monotonic()
    try:
        while True:
            try:
                return child.wait(timeout=interval)
            except subprocess.TimeoutExpired:
                heartbeat(child, started)
    finally:
        signals = [signal.SIGINT, signal.SIGTERM]
        if hasattr(signal, 'SIGBREAK'):
            signals.append(signal.SIGBREAK)
        previous = {signum: signal.signal(signum, signal.SIG_IGN) for signum in signals}
        try:
            terminate_child(child)
        finally:
            for signum, handler in previous.items():
                signal.signal(signum, handler)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--heartbeat-seconds', type=float, default=60)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command:
        parser.error('provide a command after --, for example: -- cargo test --workspace')
    if not 0 < args.heartbeat_seconds <= 3600:
        parser.error('--heartbeat-seconds must be greater than zero and at most 3600')
    previous = {}
    signals = [signal.SIGINT, signal.SIGTERM]
    if hasattr(signal, 'SIGBREAK'):
        signals.append(signal.SIGBREAK)
    for signum in signals:
        previous[signum] = signal.signal(signum, cancel)
    try:
        return run(command, args.heartbeat_seconds)
    except Cancelled as error:
        return 128 + error.signum
    except OSError as error:
        # Report the failure without echoing the full command or its potentially sensitive args.
        print(f'[build] Could not start or manage build process: {error.strerror or type(error).__name__}', file=sys.stderr, flush=True)
        return 127
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


if __name__ == '__main__':
    status = main()
    if status < 0 and os.name != 'nt':
        # Match the child signal rather than turning negative return codes into modulo-256 exits.
        signal.signal(-status, signal.SIG_DFL)
        os.kill(os.getpid(), -status)
    sys.exit(status)
