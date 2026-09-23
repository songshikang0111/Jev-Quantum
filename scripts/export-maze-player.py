#!/usr/bin/env python3
"""Bundle the maze player and a summary into a self-contained offline HTML."""
import argparse
import json
from pathlib import Path

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("report", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
report = json.loads(args.report.read_text())
embedded = json.dumps(report, ensure_ascii=True).replace("<", "\\u003c")
html = (root / "web/index.html").read_text()
html = html.replace('<link rel="stylesheet" href="styles.css">',
                    '<style>' + (root / "web/styles.css").read_text() + '</style>')
html = html.replace('<script src="last-report.js" onerror=""></script>',
                    '<script>window.JEV_EMBEDDED_REPORT=' + embedded + ';</script>')
html = html.replace('<script src="app.js"></script>',
                    '<script>' + (root / "web/app.js").read_text() + '</script>')
html = html.replace('<a href="recording.html" download="jev-maze-player.html" id="offline-download">Offline HTML</a>',
                    '<span>Offline player</span>')
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(html)
print(args.output.resolve())
