import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch


SCRIPT = Path(__file__).parents[1] / 'run_cargo.py'
SPEC = importlib.util.spec_from_file_location('run_cargo', SCRIPT)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class CargoRunnerTests(unittest.TestCase):
    def command(self, code, *arguments, interval=60):
        return [sys.executable, str(SCRIPT), '--heartbeat-seconds', str(interval),
                '--', sys.executable, '-u', '-c', code, *arguments]

    def test_child_output_is_forwarded_before_completion_and_exit_code_preserved(self):
        command = self.command(
            "import sys,time; print('ready',flush=True); time.sleep(0.5); "
            "print('child stderr',file=sys.stderr,flush=True); sys.exit(23)")
        with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as child:
            self.assertEqual(child.stdout.readline().strip(), 'ready')
            self.assertIsNone(child.poll(), 'output should arrive while the child is still running')
            _, stderr = child.communicate(timeout=15)
            self.assertEqual(child.returncode, 23)
            self.assertIn('child stderr', stderr)

    def test_arguments_are_passed_literally_without_a_shell(self):
        value = 'space ; $(not-a-command) & literal'
        result = subprocess.run(self.command('import json,sys; print(json.dumps(sys.argv[1:]))', value),
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), [value])

    def test_silent_child_receives_elapsed_heartbeat(self):
        result = subprocess.run(self.command('import time; time.sleep(0.4)', interval=0.1),
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('[build] Still running:', result.stderr)
        self.assertIn('elapsed.', result.stderr)

    def test_process_family_excludes_unrelated_compilers(self):
        rows = [dict(pid=10, ppid=1), dict(pid=11, ppid=10), dict(pid=12, ppid=11),
                dict(pid=20, ppid=1), dict(pid=21, ppid=20)]
        self.assertEqual(runner.process_family(rows, 10), {10, 11, 12})
        self.assertEqual(runner.cpu_seconds('1-02:03:04'), 93784)
        self.assertEqual(runner.cpu_seconds('03:04'), 184)

    def test_native_diagnostics_are_bounded_and_do_not_include_command_arguments(self):
        rows = [dict(pid=10, ppid=1, name='cargo.exe', cpu=2, rss=1024)]
        rows += [dict(pid=31, ppid=10, name='ta_game-test.exe', cpu=10, rss=4096)]
        rows += [dict(pid=i, ppid=10, name='rustc.exe', cpu=4, rss=2048) for i in range(11, 30)]
        rows += [dict(pid=40, ppid=1, name='unrelated-secret.exe', cpu=4, rss=2048)]
        output = Mock(returncode=0, stdout=json.dumps(rows))
        with patch.object(runner.os, 'name', 'nt'), patch.object(runner.shutil, 'which', return_value='pwsh'), \
             patch.object(runner.subprocess, 'run', return_value=output) as native:
            details = runner.native_diagnostics(10)
        self.assertEqual(len(details), runner.MAX_PROCESSES)
        self.assertIn('CPU=2.0s RSS=1MiB', details[0])
        self.assertTrue(any('ta_game-test.exe' in detail for detail in details))
        self.assertNotIn('unrelated', ' '.join(details))
        self.assertNotIn('shell', native.call_args.kwargs)
        self.assertIn('Get-Process', native.call_args.args[0][-1])
        self.assertNotIn('CommandLine', native.call_args.args[0][-1])

    def test_windows_cancellation_targets_only_own_child_tree(self):
        child = Mock(pid=123, poll=Mock(return_value=None))
        with patch.object(runner.os, 'name', 'nt'), patch.object(runner.subprocess, 'run') as taskkill:
            runner.terminate_child(child)
        self.assertEqual(taskkill.call_args.args[0], ['taskkill', '/PID', '123', '/T', '/F'])
        child.wait.assert_called_once_with(timeout=10)

    def test_unix_diagnostics_keep_long_paths_and_active_descendants(self):
        output = Mock(returncode=0, stdout=(
            ' 10 1 00:02.10 1024 /Users/runner/.rustup/toolchains/stable/bin/cargo\n'
            ' 11 10 01:04.20 2048 /Users/runner/work/a project/target/ci/deps/ta_game-tests\n'
            ' 12 1 00:59.00 4096 /unrelated/rustc\n'))
        with patch.object(runner.os, 'name', 'posix'), \
             patch.object(runner.subprocess, 'run', return_value=output) as native:
            details = runner.native_diagnostics(10)
        self.assertEqual(details, ['cargo pid=10 CPU=2.1s RSS=1MiB',
                                   'ta_game-tests pid=11 CPU=64.2s RSS=2MiB'])
        self.assertIn('-ww', native.call_args.args[0])
        self.assertTrue(native.call_args.args[0][-1].endswith(',comm='))

    @unittest.skipIf(os.name == 'nt', 'POSIX process group signals')
    def test_cancellation_does_not_stop_an_unrelated_process(self):
        with subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)']) as unrelated:
            try:
                with subprocess.Popen(self.command("import time; print('ready',flush=True); time.sleep(30)"),
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as child:
                    self.assertEqual(child.stdout.readline().strip(), 'ready')
                    child.send_signal(signal.SIGTERM)
                    child.communicate(timeout=15)
                    self.assertEqual(child.returncode, 128 + signal.SIGTERM)
                    self.assertIsNone(unrelated.poll())
            finally:
                unrelated.terminate()
                unrelated.wait(timeout=5)

    def test_invalid_interval_and_missing_command_are_rejected(self):
        for arguments in ([], ['--heartbeat-seconds', '0', '--', 'cargo']):
            result = subprocess.run([sys.executable, str(SCRIPT), *arguments], capture_output=True, timeout=15)
            self.assertEqual(result.returncode, 2)


if __name__ == '__main__':
    unittest.main()
