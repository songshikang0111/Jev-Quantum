#!/usr/bin/env python3
"""Merge offline comparisons only when the complete maze definitions match."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('reports', nargs='+', type=Path)
parser.add_argument('--output', required=True, type=Path)
parser.add_argument('--embed', type=Path, help='Optional last-report.js for the demo')
parser.add_argument('--label', action='append', default=[], metavar='SOURCE=NAME', help='Rename the sole target of a source report, preserving versioned results')
args = parser.parse_args()
reports = [json.loads(path.read_text()) for path in args.reports]
for label in args.label:
    source, sep, name = label.rpartition('=')
    matches = [r for p, r in zip(args.reports, reports) if p.resolve() == Path(source).resolve()]
    if not sep or not name or len(matches) != 1 or len(matches[0]['targets']) != 1:
        parser.error('--label needs a supplied single-target SOURCE=NAME')
    matches[0]['targets'][0]['original_target_name'] = matches[0]['targets'][0]['name']
    matches[0]['targets'][0]['name'] = name
base = reports[0]
if any(r['schema_version'] != 1 or r['scenario'] != 'maze' or r['maze'] != base['maze'] for r in reports):
    parser.error('all reports must use schema 1 and exactly the same maze')
targets = [t for r in reports for t in r['targets']]
if len({t['name'] for t in targets}) != len(targets):
    parser.error('duplicate target names; supply one run per strategy')
result = dict(base, run_id=args.output.stem.removesuffix('.summary'), targets=targets,
              started_at=min(r['started_at'] for r in reports),
              finished_at=max(r['finished_at'] for r in reports),
              config={'targets': ','.join(t['name'] for t in targets), 'merged': True,
                      'sources': [{'path': str(p), 'run_id': r['run_id'], 'config': r['config']}
                                  for p, r in zip(args.reports, reports)]})
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
if args.embed:
    args.embed.write_text('window.JEV_EMBEDDED_REPORT = ' + json.dumps(result, ensure_ascii=False) + ';\n')
print(args.output)
