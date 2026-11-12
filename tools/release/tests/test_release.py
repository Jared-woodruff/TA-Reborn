import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('release', Path(__file__).parents[1] / 'release.py')
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'incoming'
        self.source.mkdir()
        self.ver = '2.1.0'
        self.revision = 'a' * 40
        revision = patch.object(release, 'commit', return_value=self.revision)
        revision.start()
        self.addCleanup(revision.stop)

    def fixtures(self):
        for target in release.TARGETS:
            payloads, manifest, _ = release.names(self.ver, target)
            for name in payloads:
                (self.source / name).write_bytes(('fixture:' + name).encode())
            (self.source / manifest).write_text(''.join(
                f'{release.digest(self.source / name)}  {name}\n' for name in payloads), encoding='ascii')
            release.record(self.source, self.ver, target, self.revision)

    def test_assembly_requires_all_six_matching_targets_and_preserves_x64_legacy_update(self):
        self.fixtures()
        output = self.root / 'publish'
        names = release.assemble(self.source, output, self.ver, self.revision)
        self.assertEqual(len(release.TARGETS), 6)
        self.assertEqual(len(names), 34)  # 20 packages, 6 manifests, 6 records, alias, global sums
        self.assertEqual((output / 'TA-Reborn-2.1.0-setup.exe').read_bytes(),
                         (output / 'TA-Reborn-2.1.0-windows-x64-setup.exe').read_bytes())
        self.assertNotEqual((output / 'TA-Reborn-2.1.0-setup.exe').read_bytes(),
                            (output / 'TA-Reborn-2.1.0-windows-arm64-setup.exe').read_bytes())
        for line in (output / 'TA-Reborn-2.1.0-SHA256SUMS.txt').read_text().splitlines():
            sha, name = line.split('  ', 1)
            self.assertEqual(sha, release.digest(output / name))

    def test_missing_or_changed_payload_prevents_any_publication_output(self):
        self.fixtures()
        payload = self.source / 'TA-Reborn-2.1.0-linux-arm64-ubuntu.deb'
        payload.write_bytes(b'changed after the build')
        with self.assertRaisesRegex(ValueError, 'Checksum mismatch'):
            release.assemble(self.source, self.root / 'publish', self.ver, self.revision)
        self.assertFalse((self.root / 'publish').exists())
        payload.unlink()
        with self.assertRaisesRegex(ValueError, 'Missing'):
            release.assemble(self.source, self.root / 'publish', self.ver, self.revision)

    def test_stale_revision_and_unexpected_assets_are_rejected(self):
        self.fixtures()
        with self.assertRaisesRegex(ValueError, 'revision'):
            release.assemble(self.source, self.root / 'publish', self.ver, 'b' * 40)
        (self.source / 'TA-Reborn-old-setup.exe').write_bytes(b'stale')
        with self.assertRaisesRegex(ValueError, 'unexpected'):
            release.assemble(self.source, self.root / 'publish', self.ver, self.revision)

    def test_checksum_manifest_cannot_omit_symbols_or_repeat_an_entry(self):
        self.fixtures()
        target = 'x86_64-pc-windows-msvc'
        _, name, _ = release.names(self.ver, target)
        manifest = self.source / name
        lines = manifest.read_text().splitlines()
        manifest.write_text('\n'.join(lines[:-1]) + '\n')
        with self.assertRaisesRegex(ValueError, 'exactly'):
            release.record(self.source, self.ver, target, self.revision)
        manifest.write_text('\n'.join(lines + [lines[0]]) + '\n')
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            release.record(self.source, self.ver, target, self.revision)

    def test_release_tag_must_match_workspace_version_but_manual_branch_build_is_allowed(self):
        release.validate_event(self.ver, 'workflow_dispatch', {}, 'refs/heads/test')
        event = {'action': 'published', 'release': {'tag_name': 'v2.1.0', 'draft': False}}
        release.validate_event(self.ver, 'release', event, 'refs/tags/v2.1.0')
        for bad in ['v2.0.0', 'latest', 'v2.1.0\nmalicious']:
            event['release']['tag_name'] = bad
            with self.assertRaises(ValueError):
                release.validate_event(self.ver, 'release', event, '')

    def test_assembly_never_overwrites_a_previous_output(self):
        output = self.root / 'publish'
        output.mkdir()
        with self.assertRaisesRegex(ValueError, 'already exists'):
            release.assemble(self.source, output, self.ver, self.revision)

    def test_publisher_does_not_clobber_existing_or_immutable_assets(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        for state in [dict(isDraft=False, isImmutable=True, assets=[]),
                      dict(isDraft=False, isImmutable=False, assets=[]),
                      dict(isDraft=True, isImmutable=False, assets=[{'name': 'TA-Reborn-2.1.0-setup.exe'}]),
                      dict(isDraft=True, isImmutable=False, assets=[{'name': 'another-setup.exe'}]),
                      dict(isDraft=True, isImmutable=False, assets=[{'name': 'another-SHA256SUMS.txt'}])]:
            with patch.object(release.subprocess, 'check_output', return_value=json.dumps(state)), \
                 patch.object(release.subprocess, 'run') as upload:
                with self.assertRaises(ValueError):
                    release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
                upload.assert_not_called()

    def test_legacy_checksums_and_x64_setup_are_uploaded_before_concurrent_archives(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        for draft in (True,):
            state = dict(isDraft=draft, isImmutable=False, assets=[])
            with patch.object(release.subprocess, 'check_output', return_value=json.dumps(state)), \
                 patch.object(release.subprocess, 'run') as upload:
                release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
                calls = [call.args[0] for call in upload.call_args_list]
                self.assertEqual(len(calls), 3)
                self.assertEqual(calls[0][-1], str(folder / 'TA-Reborn-2.1.0-SHA256SUMS.txt'))
                self.assertEqual(calls[1][-1], str(folder / 'TA-Reborn-2.1.0-setup.exe'))
                self.assertEqual(len(calls[0]), 7)
                self.assertEqual(len(calls[1]), 7)
                self.assertIn(str(folder / 'TA-Reborn-2.1.0-windows-arm64-setup.exe'), calls[2])
                self.assertTrue(all('--clobber' not in call for call in calls))

    def test_publisher_rechecks_payloads_before_uploading(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        (folder / 'TA-Reborn-2.1.0-windows-arm64-setup.exe').write_bytes(b'changed after assembly')
        with patch.object(release.subprocess, 'run') as upload:
            with self.assertRaisesRegex(ValueError, 'Checksum mismatch'):
                release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
            upload.assert_not_called()

    def test_publisher_requires_build_records_from_current_checkout(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        with patch.object(release, 'commit', return_value='b' * 40), \
             patch.object(release.subprocess, 'run') as upload:
            with self.assertRaisesRegex(ValueError, 'revision'):
                release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
            upload.assert_not_called()

    def test_publisher_rechecks_every_target_record(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        _, _, name = release.names(self.ver, 'aarch64-unknown-linux-gnu')
        record = folder / name
        original = record.read_text()
        data = json.loads(original)
        data['target'] = 'x86_64-unknown-linux-gnu'
        record.write_text(json.dumps(data))
        with patch.object(release.subprocess, 'run') as upload:
            with self.assertRaisesRegex(ValueError, 'Build record'):
                release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
            upload.assert_not_called()

    def test_publisher_requires_complete_valid_global_checksums(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        manifest = folder / 'TA-Reborn-2.1.0-SHA256SUMS.txt'
        lines = manifest.read_text().splitlines()
        changes = [
            (lines[:-1], 'exactly'),
            (lines + [lines[0]], 'duplicate'),
            (['0' * 64 + lines[0][64:]] + lines[1:], 'Global checksum mismatch'),
        ]
        for changed, error in changes:
            with self.subTest(error=error), patch.object(release.subprocess, 'run') as upload:
                manifest.write_text('\n'.join(changed) + '\n')
                with self.assertRaisesRegex(ValueError, error):
                    release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
                upload.assert_not_called()

    def test_publisher_rejects_arm64_legacy_alias_even_with_matching_global_checksum(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        alias = folder / 'TA-Reborn-2.1.0-setup.exe'
        original_hash = release.digest(alias)
        alias.write_bytes((folder / 'TA-Reborn-2.1.0-windows-arm64-setup.exe').read_bytes())
        manifest = folder / 'TA-Reborn-2.1.0-SHA256SUMS.txt'
        manifest.write_text(manifest.read_text().replace(
            f'{original_hash}  {alias.name}', f'{release.digest(alias)}  {alias.name}'))
        with patch.object(release.subprocess, 'run') as upload:
            with self.assertRaisesRegex(ValueError, 'Legacy setup must be identical'):
                release.publish(folder, self.ver, 'owner/repo', 'v2.1.0')
            upload.assert_not_called()

    def uploaded(self, folder, **changes):
        assets = [dict(name=p.name, size=p.stat().st_size, state='uploaded') for p in sorted(folder.iterdir())]
        for asset in assets:
            asset.update(changes.get(asset['name'], {}))
        return assets

    def test_the_build_needs_an_empty_draft_for_its_tag(self):
        for state, error in [(dict(isDraft=False, assets=[], name='TA Reborn 2.1.0'), 'already published'),
                             (dict(isDraft=True, assets=[{'name': 'x'}], name='TA Reborn 2.1.0'), 'already holds files')]:
            with patch.object(release.subprocess, 'check_output', return_value=json.dumps(state)):
                with self.assertRaisesRegex(ValueError, error):
                    release.require_draft(self.ver, 'owner/repo', 'v2.1.0')
        with patch.object(release.subprocess, 'check_output', side_effect=release.subprocess.CalledProcessError(1, 'gh')):
            with self.assertRaisesRegex(ValueError, 'create its draft'):
                release.require_draft(self.ver, 'owner/repo', 'v2.1.0')
        with self.assertRaisesRegex(ValueError, 'does not match'):
            release.require_draft(self.ver, 'owner/repo', 'v2.2.0')
        state = dict(isDraft=True, assets=[], name='TA Reborn 2.1.0')
        with patch.object(release.subprocess, 'check_output', return_value=json.dumps(state)):
            self.assertEqual(release.require_draft(self.ver, 'owner/repo', 'v2.1.0'), 'TA Reborn 2.1.0')

    def test_a_complete_draft_is_published_before_the_previous_release_is_deleted(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        draft = json.dumps(dict(isDraft=True, assets=self.uploaded(folder)))
        listed = json.dumps([dict(tagName='v2.1.0', isLatest=False), dict(tagName='v2.0.9', isLatest=True), dict(tagName='v2.0.8', isLatest=False)])
        answers = [draft, listed, json.dumps(dict(isDraft=False))]
        with patch.object(release.subprocess, 'check_output', side_effect=answers), \
             patch.object(release.subprocess, 'run') as run:
            self.assertEqual(release.promote(folder, self.ver, 'owner/repo', 'v2.1.0'), ['v2.0.9'])
        calls = [call.args[0] for call in run.call_args_list]
        self.assertEqual(calls[0][:4], ['gh', 'release', 'edit', 'v2.1.0'])
        self.assertIn('--draft=false', calls[0])
        self.assertIn('--latest', calls[0])
        self.assertEqual(calls[1][:4], ['gh', 'release', 'delete', 'v2.0.9'])
        self.assertNotIn('--cleanup-tag', calls[1])
        self.assertEqual(len(calls), 2, 'only the previous latest release goes')

    def test_an_incomplete_or_published_draft_is_not_promoted(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        setup = 'TA-Reborn-2.1.0-windows-x64-setup.exe'
        for state, error in [(dict(isDraft=True, assets=self.uploaded(folder)[1:]), 'exactly the release set'),
                             (dict(isDraft=True, assets=self.uploaded(folder, **{setup: dict(size=1)})), 'Incomplete upload'),
                             (dict(isDraft=True, assets=self.uploaded(folder, **{setup: dict(state='starter')})), 'Incomplete upload'),
                             (dict(isDraft=False, assets=self.uploaded(folder)), 'not a draft')]:
            with patch.object(release.subprocess, 'check_output', return_value=json.dumps(state)), \
                 patch.object(release.subprocess, 'run') as run:
                with self.assertRaisesRegex(ValueError, error):
                    release.promote(folder, self.ver, 'owner/repo', 'v2.1.0')
                run.assert_not_called()

    def test_the_first_release_has_no_previous_one_to_delete(self):
        self.fixtures()
        folder = self.root / 'publish'
        release.assemble(self.source, folder, self.ver, self.revision)
        answers = [json.dumps(dict(isDraft=True, assets=self.uploaded(folder))), json.dumps([]), json.dumps(dict(isDraft=False))]
        with patch.object(release.subprocess, 'check_output', side_effect=answers), \
             patch.object(release.subprocess, 'run') as run:
            self.assertEqual(release.promote(folder, self.ver, 'owner/repo', 'v2.1.0'), [])
        self.assertEqual(len(run.call_args_list), 1)

    def test_workspace_version_is_read_from_the_correct_section(self):
        (self.root / 'Cargo.toml').write_text('[package]\nversion="0.1.0"\n[workspace.package]\nversion = "2.1.0-rc.1"\n')
        self.assertEqual(release.version(self.root), '2.1.0-rc.1')


if __name__ == '__main__':
    unittest.main()
