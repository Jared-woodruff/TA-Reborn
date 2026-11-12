#!/usr/bin/env python3
"""Validate and assemble the six release jobs, attach them to the release's draft and publish it
once complete, then retire the previous release; never silently replace published files."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
TARGETS = {
    'x86_64-pc-windows-msvc': ('windows-x64', ('-setup.exe', '.zip', '-symbols.zip')),
    'aarch64-pc-windows-msvc': ('windows-arm64', ('-setup.exe', '.zip', '-symbols.zip')),
    'x86_64-apple-darwin': ('macos-x64', ('.zip', '.dmg', '-symbols.tar.gz')),
    'aarch64-apple-darwin': ('macos-arm64', ('.zip', '.dmg', '-symbols.tar.gz')),
    'x86_64-unknown-linux-gnu': ('linux-x64', ('.tar.gz', '-ubuntu.deb', '-arch.pkg.tar.zst', '-symbols.tar.gz')),
    'aarch64-unknown-linux-gnu': ('linux-arm64', ('.tar.gz', '-ubuntu.deb', '-arch.pkg.tar.zst', '-symbols.tar.gz')),
}


def version(root=ROOT):
    text = (root / 'Cargo.toml').read_text(encoding='utf-8')
    section = re.search(r'(?ms)^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)', text)
    match = re.search(r'(?m)^version\s*=\s*"([^"\r\n]+)"', section[1] if section else '')
    if not match or not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?', match[1]):
        raise ValueError('Cargo.toml must contain a valid workspace release version')
    return match[1]


def commit():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def validate_event(ver, event_name, event, ref):
    tag = event.get('release', {}).get('tag_name') if event_name == 'release' else None
    if event_name == 'release' and (not tag or event.get('action') != 'published' or event['release'].get('draft')):
        raise ValueError('Release publication requires a published, non-draft release')
    if tag is None and ref.startswith('refs/tags/'):
        tag = ref.removeprefix('refs/tags/')
    if tag is not None and tag not in (ver, 'v' + ver):
        raise ValueError(f'Release tag {tag!r} does not match Cargo.toml version {ver}')


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def names(ver, target):
    platform, suffixes = TARGETS[target]
    prefix = f'TA-Reborn-{ver}-{platform}'
    return [prefix + s for s in suffixes], prefix + '-SHA256SUMS.txt', prefix + '-build.json'


def checked_file(folder, name):
    if Path(name).name != name or '/' in name or '\\' in name:
        raise ValueError(f'Unsafe asset name: {name}')
    path = folder / name
    if path.is_symlink() or not path.is_file() or path.resolve().parent != folder.resolve():
        raise ValueError(f'Missing, linked or misplaced release asset: {path}')
    if path.stat().st_size == 0:
        raise ValueError(f'Empty release asset: {path}')
    return path


def read_checksums(folder, sums, expected_names):
    listed = {}
    for line in checked_file(folder, sums).read_text(encoding='utf-8-sig').splitlines():
        match = re.fullmatch(r'([0-9a-fA-F]{64}) [ *](.+)', line)
        if not match or match[2] in listed:
            raise ValueError(f'Invalid or duplicate checksum entry in {sums}')
        listed[match[2]] = match[1].lower()
    if set(listed) != set(expected_names):
        raise ValueError(f'{sums} must cover exactly {sorted(expected_names)}')
    return listed


def validate_group(folder, ver, target):
    payloads, sums, _ = names(ver, target)
    listed = read_checksums(folder, sums, payloads)
    for name, expected in listed.items():
        if digest(checked_file(folder, name)) != expected:
            raise ValueError(f'Checksum mismatch: {name}')
    return {**listed, sums: digest(checked_file(folder, sums))}


def record(folder, ver, target, revision):
    hashes = validate_group(folder, ver, target)
    _, _, name = names(ver, target)
    data = dict(schema=1, version=ver, target=target, commit=revision, files=hashes)
    (folder / name).write_text(json.dumps(data, indent=2) + '\n', encoding='utf-8', newline='\n')


def validate_build_records(source, ver, revision):
    files = {}
    for target in TARGETS:
        hashes = validate_group(source, ver, target)
        _, _, record_name = names(ver, target)
        entry = json.loads(checked_file(source, record_name).read_text(encoding='utf-8'))
        if entry != dict(schema=1, version=ver, target=target, commit=revision, files=hashes):
            raise ValueError(f'Build record does not match this revision and its artifacts: {record_name}')
        files.update(hashes)
        files[record_name] = digest(source / record_name)
    return files


def assemble(source, destination, ver, revision):
    # A fresh output directory also prevents stale packages from older runs leaking in.
    if destination.exists():
        raise ValueError(f'Assembly output already exists: {destination}')
    files = validate_build_records(source, ver, revision)
    if {p.name for p in source.iterdir()} != set(files):
        raise ValueError('Input contains unexpected files or directories; refusing an ambiguous release')
    destination.mkdir(parents=True)
    for name in files:
        shutil.copyfile(checked_file(source, name), destination / name)
    # Existing x64 launchers search for the historical setup filename. ARM64 clients
    # select their exact architecture and never consume this compatibility alias.
    legacy = f'TA-Reborn-{ver}-setup.exe'
    shutil.copyfile(destination / f'TA-Reborn-{ver}-windows-x64-setup.exe', destination / legacy)
    files[legacy] = digest(destination / legacy)
    sums = destination / f'TA-Reborn-{ver}-SHA256SUMS.txt'
    sums.write_text(''.join(f'{sha}  {name}\n' for name, sha in sorted(files.items())), encoding='ascii', newline='\n')
    return sorted(files) + [sums.name]


def expected_assets(ver):
    """The complete release set: every target's files, the x64 alias and the global checksums."""
    expected = {f'TA-Reborn-{ver}-setup.exe', f'TA-Reborn-{ver}-SHA256SUMS.txt'}
    for target in TARGETS:
        payloads, manifest, build_record = names(ver, target)
        expected.update(payloads + [manifest, build_record])
    return expected


def release_state(repository, tag, fields):
    # (gh finds a draft by its tag too, which the API's lookup by tag does not.)
    return json.loads(subprocess.check_output(['gh', 'release', 'view', tag, '--repo', repository, '--json', fields], text=True))


def require_draft(ver, repository, tag):
    """The tag's release must be a draft with nothing attached yet: the packages go to the draft,
    which is published once complete, and the previous release stays published meanwhile."""
    if tag not in (ver, 'v' + ver):
        raise ValueError(f'Release tag {tag!r} does not match Cargo.toml version {ver}')
    try:
        state = release_state(repository, tag, 'assets,isDraft,name')
    except subprocess.CalledProcessError as error:
        raise ValueError(f'No release for {tag}: create its draft with its notes first (gh release create {tag} --draft ...)') from error
    if not state.get('isDraft'):
        raise ValueError(f'The release for {tag} is already published: packages are attached to its draft, published once complete')
    if state.get('assets'):
        raise ValueError(f'The draft for {tag} already holds files; review and remove them before building it again')
    return state.get('name') or tag


def publish(folder, ver, repository, tag):
    if tag not in (ver, 'v' + ver):
        raise ValueError('Publish tag does not match the workspace version')
    paths = sorted(folder.iterdir())
    legacy = f'TA-Reborn-{ver}-setup.exe'
    sums = f'TA-Reborn-{ver}-SHA256SUMS.txt'
    expected = expected_assets(ver)
    if {p.name for p in paths} != expected:
        raise ValueError('Publish requires the complete assembled release set')
    # Recheck unpacked/downloaded artifacts at publication, including manual draft uploads.
    # Matching names alone cannot establish that they still belong to this checkout.
    files = validate_build_records(folder, ver, commit())
    files[legacy] = digest(checked_file(folder, legacy))
    if files[legacy] != files[f'TA-Reborn-{ver}-windows-x64-setup.exe']:
        raise ValueError('Legacy setup must be identical to the Windows x64 installer')
    global_hashes = read_checksums(folder, sums, files)
    for name, actual in files.items():
        if global_hashes[name] != actual:
            raise ValueError(f'Global checksum mismatch: {name}')
    release = release_state(repository, tag, 'assets,isDraft,isImmutable')
    if release.get('isImmutable'):
        raise ValueError('Cannot attach packages to an immutable release; attach files to its draft before publication')
    if not release.get('isDraft'):
        raise ValueError('Packages are attached to the release\'s draft, which is published only once complete')
    existing = {asset['name'] for asset in release['assets']}
    overlap = existing.intersection(p.name for p in paths)
    if overlap:
        raise ValueError(f'Release already contains these assets; review them before retrying: {sorted(overlap)}')
    if any(name.lower().endswith(('-setup.exe', '-sha256sums.txt')) for name in existing):
        raise ValueError('Existing setup/checksum assets would redirect legacy launchers; review the release first')
    for path in paths:
        checked_file(folder, path.name)
    command = ['gh', 'release', 'upload', tag, '--repo', repository]
    # Legacy clients pick the FIRST suffix match. gh uploads a list concurrently,
    # so establish the global checksums and x64 alias before any per-target files.
    subprocess.run(command + [str(folder / sums)], check=True)
    subprocess.run(command + [str(folder / legacy)], check=True)
    remaining = [str(p) for p in paths if p.name not in (sums, legacy)]
    subprocess.run(command + remaining, check=True)


def version_key(tag):
    """A release tag's version, for ordering ('v2.1.0' < 'v2.1.1'; a pre-release before its
    release), or None for a tag that names no version."""
    match = re.fullmatch(r'v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?', tag)
    if not match:
        return None
    major, minor, patch, pre = match.groups()
    return (int(major), int(minor), int(patch), 0 if pre else 1, pre or '')


def promote(folder, ver, repository, tag):
    """Publishes the tag's draft as the latest release once it holds exactly the assembled set,
    each file at its full size, and only then deletes the release that was the latest before it
    (its tag stays): until the new release is whole, players keep downloading the previous one.
    Returns the tags of the releases deleted."""
    if tag not in (ver, 'v' + ver):
        raise ValueError('Promote tag does not match the workspace version')
    expected = expected_assets(ver)
    local = {p.name: p.stat().st_size for p in folder.iterdir()}
    if set(local) != expected:
        raise ValueError('Promotion requires the complete assembled release set')
    state = release_state(repository, tag, 'assets,isDraft')
    if not state.get('isDraft'):
        raise ValueError(f'The release for {tag} is not a draft; it was published before its packages were complete')
    uploaded = {asset['name']: asset for asset in state.get('assets', [])}
    if set(uploaded) != expected:
        raise ValueError(f'The draft must hold exactly the release set: missing {sorted(expected - set(uploaded))}, '
                         f'unexpected {sorted(set(uploaded) - expected)}')
    for name, size in local.items():
        asset = uploaded[name]
        if asset.get('size') != size or asset.get('state', 'uploaded') != 'uploaded':
            raise ValueError(f'Incomplete upload in the draft: {name}')
    releases = json.loads(subprocess.check_output(
        ['gh', 'release', 'list', '--repo', repository, '--exclude-drafts', '--json', 'tagName,isLatest', '--limit', '100'], text=True))
    previous = [r['tagName'] for r in releases if r.get('isLatest') and r.get('tagName') != tag]
    # (A build of an older version finishing after a newer one's must not take its place as the
    # latest, nor delete it: the draft then stays a draft, for a person to decide.)
    for old in previous:
        theirs, mine = version_key(old), version_key(tag)
        if theirs is None or mine is None or theirs >= mine:
            raise ValueError(f'The latest release, {old}, is not older than {tag}: {tag} stays a draft, and {old} stays published')
    subprocess.run(['gh', 'release', 'edit', tag, '--repo', repository, '--draft=false', '--latest'], check=True)
    if release_state(repository, tag, 'isDraft').get('isDraft'):
        raise ValueError(f'The release for {tag} is still a draft after publishing it')
    for old in previous:
        # (Without --cleanup-tag: the old release's tag stays.)
        subprocess.run(['gh', 'release', 'delete', old, '--repo', repository, '--yes'], check=True)
    return previous


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('validate')
    make_record = sub.add_parser('record')
    make_record.add_argument('--target', choices=TARGETS, required=True)
    make_record.add_argument('--dir', type=Path, required=True)
    collect = sub.add_parser('assemble')
    collect.add_argument('--source', type=Path, required=True)
    collect.add_argument('--output', type=Path, required=True)
    upload = sub.add_parser('publish')
    upload.add_argument('--dir', type=Path, required=True)
    sub.add_parser('draft')
    finish = sub.add_parser('promote')
    finish.add_argument('--dir', type=Path, required=True)
    args = parser.parse_args()
    ver = version()
    if args.command == 'validate':
        event_path = os.environ.get('GITHUB_EVENT_PATH')
        event = json.loads(Path(event_path).read_text(encoding='utf-8')) if event_path else {}
        validate_event(ver, os.environ.get('GITHUB_EVENT_NAME', ''), event, os.environ.get('GITHUB_REF', ''))
        print(f'Building TA Reborn {ver} from {commit()}')
    elif args.command == 'record':
        record(args.dir, ver, args.target, commit())
    elif args.command == 'assemble':
        for name in assemble(args.source, args.output, ver, commit()):
            print(name)
    elif args.command == 'publish':
        publish(args.dir, ver, os.environ['GITHUB_REPOSITORY'], os.environ['RELEASE_TAG'])
    elif args.command == 'draft':
        name = require_draft(ver, os.environ['GITHUB_REPOSITORY'], os.environ['RELEASE_TAG'])
        print(f'Building {name} into its draft')
    elif args.command == 'promote':
        retired = promote(args.dir, ver, os.environ['GITHUB_REPOSITORY'], os.environ['RELEASE_TAG'])
        print(f"Published {os.environ['RELEASE_TAG']}; deleted the previous release: {', '.join(retired) or 'none'} (tags kept)")


if __name__ == '__main__':
    main()
