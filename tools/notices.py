#!/usr/bin/env python3
"""Collect shipped dependency license files without recording local cache paths."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def notices():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=ROOT, text=True))
    packages = sorted((p for p in metadata['packages'] if p['source']),
                      key=lambda p: (p['name'], p['version']))
    documents = {}
    entries = []
    for package in packages:
        if package['source'] != 'registry+https://github.com/rust-lang/crates.io-index' or not package['license']:
            raise ValueError(f"unreviewed dependency source or license: {package['name']}")
        root = Path(package['manifest_path']).parent
        references = []
        for path in sorted(root.rglob('*')):
            if path.is_symlink() or not path.is_file():
                continue
            if not path.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE', 'COPYRIGHT')):
                continue
            content = path.read_text(encoding='utf-8')
            digest = hashlib.sha256(content.encode()).hexdigest()
            documents[digest] = content
            references.append(f'{path.relative_to(root).as_posix()}: {digest}')
        entries.append(f"\n{package['name']} {package['version']} — {package['license']}\n"
                       f"Source: https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download\n"
                       + ('\n'.join(references) if references else
                          'No separate license file shipped in the upstream crate; see its source archive and the license texts below.') + '\n')
    # These crates inherit the generator's MPL license without shipping their own file.
    mpl = next(p for p in packages if p['name'] == 'progenitor')
    content = (Path(mpl['manifest_path']).parent / 'LICENSE').read_text()
    documents[hashlib.sha256(content.encode()).hexdigest()] = content
    header = ('Third-party notices for Anywhere\n\n'
              'This inventory includes build and test dependencies as well as runtime dependencies.\n'
              'Original license/copyright files below are retained verbatim, deduplicated by SHA-256.\n'
              'Source archives for every exact dependency version are linked below.\n'
              'For MPL-2.0 components, these archives provide their unmodified covered source.\n'
              'Regenerate with mise exec -- just notices after changing Cargo.lock.\n'
              f'Cargo.lock SHA-256: {hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest()}\n')
    return header + ''.join(entries) + ''.join(
        f'\n===== License document {digest} =====\n{content}\n'
        for digest, content in sorted(documents.items()))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    generated = notices()
    destination = ROOT / 'THIRD_PARTY_NOTICES.txt'
    if args.check:
        if destination.read_text() != generated:
            raise SystemExit('dependency notices are stale; run just notices')
    else:
        destination.write_text(generated)
