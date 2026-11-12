# SPDX-License-Identifier: MIT
"""Compare existing render fixtures between two committed revisions byte for byte."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


SNAPSHOT = r'''
#[cfg(test)]
mod render_comparison {
    use super::*;

    #[test]
    fn snapshot_existing_renders() {
        let root = std::path::PathBuf::from(std::env::var_os("RENDER_COMPARISON_ROOT").unwrap());
        for fixture in ["pdf", "text_pdf", "sample"] {
            let pdf = load(std::fs::read(root.join(format!("inputs/{fixture}.pdf"))).unwrap()).unwrap();
            for (page, target) in pdf.pages().iter().enumerate() {
                for scale in [1.0, 2.0833333] {
                    for (background, bg_color) in [("white", WHITE), ("transparent", AlphaColor::TRANSPARENT)] {
                        for (format, name) in [(Format::Png, "png"), (Format::Jpeg, "jpeg"), (Format::Webp, "webp")] {
                            let settings = Settings { scale, format, quality: 90, bg_color };
                            let rs = RenderSettings { x_scale: scale, y_scale: scale, bg_color, ..Default::default() };
                            let bytes = encode(render(target, &RenderCache::new(), &font_settings(), &rs), &settings).unwrap();
                            std::fs::write(root.join(format!("renders/{fixture}-{page}-{scale}-{background}.{name}")), bytes).unwrap();
                        }
                    }
                }
            }
        }
    }
}
'''


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args])


def digest(data):
    return hashlib.sha256(data).hexdigest()


def snapshot(repo, revision, directory, target):
    directory.mkdir()
    archive = git(repo, 'archive', revision, 'pdf2image/component',
                  'pdf2image/src/pdf2image/fixtures.lua')
    subprocess.run(['tar', '-xf', '-', '-C', str(directory)], input=archive, check=True)
    component = directory / 'pdf2image/component'
    inputs = directory / 'inputs'
    inputs.mkdir()
    (directory / 'renders').mkdir()
    (inputs / 'sample.pdf').write_bytes((component / 'tests/assets/sample.pdf').read_bytes())
    lua = (directory / 'pdf2image/src/pdf2image/fixtures.lua').read_text()
    for name in ['pdf', 'text_pdf']:
        encoded = re.search(r'\b' + name + r'\s*=\s*base64.decode\("([^"]+)"\)', lua)
        if encoded is None:
            raise ValueError(f'Missing fixture: {name}')
        (inputs / f'{name}.pdf').write_bytes(base64.b64decode(encoded[1], validate=True))
    source = component / 'src/lib.rs'
    source.write_text(source.read_text() + SNAPSHOT)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), RENDER_COMPARISON_ROOT=str(directory))
    with (directory / 'cargo.log').open('w') as log:
        subprocess.run(['cargo', 'test', '--locked', 'render_comparison::snapshot_existing_renders',
                        '--', '--exact'], cwd=component, env=env, stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    outputs = {p.name: p.read_bytes() for p in (directory / 'renders').iterdir()}
    if len(outputs) != 48:
        raise ValueError(f'Expected 48 renders, got {len(outputs)}; see {directory / "cargo.log"}')
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', default='origin/main')
    parser.add_argument('--candidate', default='HEAD')
    parser.add_argument('--work-dir', required=True, type=Path, help='New directory for sources, builds, and report')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[3]
    refs = {name: git(repo, 'rev-parse', '--verify', f'{ref}^{{commit}}').decode().strip()
            for name, ref in [('baseline', args.baseline), ('candidate', args.candidate)]}
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    outputs = {name: snapshot(repo, ref, root / name, root / 'target') for name, ref in refs.items()}
    if outputs['baseline'].keys() != outputs['candidate'].keys():
        raise ValueError('Render sets differ')
    rows = [{'file': name, 'bytes': len(data), 'sha256': digest(data),
             'candidate_sha256': digest(outputs['candidate'][name]),
             'identical': data == outputs['candidate'][name]}
            for name, data in sorted(outputs['baseline'].items())]
    manifest = [{k: row[k] for k in ('file', 'bytes', 'sha256')} for row in rows]
    report = {'revisions': refs, 'rustc': subprocess.check_output(['rustc', '--version']).decode().strip(),
              'scales': [1.0, 2.0833333], 'backgrounds': ['white', 'transparent'],
              'formats': ['png', 'jpeg', 'webp'], 'jpeg_quality': 90,
              'manifest_sha256': digest(json.dumps(manifest, sort_keys=True).encode()), 'renders': rows}
    (root / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n')
    if not all(row['identical'] for row in rows):
        raise SystemExit('Render bytes differ; see comparison.json')
    print(f"{len(rows)} byte-identical renders; manifest SHA-256 {report['manifest_sha256']}")


if __name__ == '__main__':
    main()
