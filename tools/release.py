#!/usr/bin/env python3
"""Stage CLI-only release archives locally. Never upload or publish."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {'aarch64-apple-darwin', 'x86_64-apple-darwin', 'x86_64-unknown-linux-musl'}


def skill_files():
    """Only the public skill entrypoint and regular Markdown references are releasable."""
    root = ROOT / 'skills/anywhere'
    files = []
    for path in sorted(root.rglob('*')):
        relative = path.relative_to(root)
        allowed = relative == Path('SKILL.md') or relative.parts[0] == 'references'
        if path.is_symlink() or not allowed:
            raise ValueError(f'unsupported skill bundle entry: {relative}')
        if path.is_dir():
            continue
        if not path.is_file() or path.suffix != '.md':
            raise ValueError(f'unsupported skill bundle file: {relative}')
        files.append((path, f'skills/anywhere/{relative.as_posix()}'))
    if not any(name == 'skills/anywhere/SKILL.md' for _, name in files):
        raise ValueError('the skill bundle needs SKILL.md')
    return files


def stage(target):
    if target not in TARGETS:
        raise ValueError('unsupported installer target')
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
        raise ValueError('commit the reviewed source before staging release archives')
    subprocess.run([sys.executable, str(ROOT / 'tools/notices.py'), '--check'], check=True)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    version = tomllib.loads((ROOT / 'cli/Cargo.toml').read_text())['package']['version']
    if version == '0.0.0':
        raise ValueError('choose a release version in cli/Cargo.toml first')
    environment = os.environ.copy()
    environment["ANYWHERE_SOURCE_REVISION"] = revision
    if target.endswith('apple-darwin'):
        environment['MACOSX_DEPLOYMENT_TARGET'] = '13.0'
    else:
        environment['CC_x86_64_unknown_linux_musl'] = 'musl-gcc'
        environment['CFLAGS_x86_64_unknown_linux_musl'] = '-march=x86-64'
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'anywhere',
                    '--target', target, '--target-dir', str(ROOT / 'target')], cwd=ROOT, env=environment, check=True)
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
        raise ValueError('source changed while building; review and rebuild')
    binary = ROOT / 'target' / target / 'release/anywhere'
    destination = ROOT / 'dist'
    destination.mkdir(exist_ok=True)
    archive = destination / f'anywhere-{version}-{target}.tar.gz'
    if archive.exists() or archive.with_suffix(archive.suffix + '.sha256').exists():
        raise ValueError(f'archive already exists: {archive.name}')
    # Fixed payload: no recursive copy of a platform bundle, checkout, or build tree.
    with tempfile.TemporaryDirectory(prefix='anywhere-release-', dir=destination) as temporary:
        work = Path(temporary)
        (work / 'SOURCE').write_text(f'https://github.com/anywhere-so/anywhere\ncommit {revision}\n')
        staged_archive = work / archive.name
        with tarfile.open(staged_archive, 'w:gz') as stream:
            for path, name in [(binary, 'anywhere'), (ROOT / 'LICENSE', 'LICENSE'),
                               (ROOT / 'THIRD_PARTY_NOTICES.txt', 'THIRD_PARTY_NOTICES.txt'),
                               (work / 'SOURCE', 'SOURCE'), *skill_files()]:
                stream.add(path, arcname=name, recursive=False)
        digest = hashlib.sha256(staged_archive.read_bytes()).hexdigest()
        staged_checksum = work / (archive.name + '.sha256')
        staged_checksum.write_text(f'{digest}  {archive.name}\n')
        staged_archive.rename(archive)
        staged_checksum.rename(archive.with_suffix(archive.suffix + '.sha256'))
    print(archive)


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit('usage: release.py TARGET')
    try:
        stage(sys.argv[1])
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f'release: {error}')
