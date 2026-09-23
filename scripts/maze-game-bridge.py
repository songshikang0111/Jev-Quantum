#!/usr/bin/env python3
"""Expose only local game observations/actions to a player; no source/map endpoints."""
import argparse
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--port', type=int, default=3187)
parser.add_argument('--width', type=int, default=10)
parser.add_argument('--height', type=int, default=10)
parser.add_argument('--maze-seed', type=int, required=True)
parser.add_argument('--max-runtime-secs', type=int, default=600)
parser.add_argument('--resume-report', type=Path)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
cmd = ['target/release/jev-quantum-bench', 'maze-session', '--width', str(args.width),
       '--height', str(args.height), '--maze-seed', str(args.maze_seed),
       '--max-runtime-secs', str(args.max_runtime_secs), '--output', str(args.output)]
if args.resume_report:
    cmd += ['--resume-report', str(args.resume_report)]
proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
current = json.loads(proc.stdout.readline())
done = False

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.respond(current if self.path == '/observation' else {'error': 'unknown endpoint'},
                     200 if self.path == '/observation' else 404)

    def do_POST(self):
        global current, done
        if self.path != '/move':
            self.respond({'error': 'unknown endpoint'}, 404)
            return
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 100:
                raise ValueError()
            data = json.loads(self.rfile.read(length))
            if set(data) != {'action'} or data['action'] not in ('UP', 'RIGHT', 'DOWN', 'LEFT'):
                raise ValueError()
        except (ValueError, TypeError):
            self.respond({'error': 'send exactly one action: UP RIGHT DOWN LEFT'}, 400)
            return
        if proc.poll() is not None:
            self.respond({'event': 'finished', 'reason': 'broker stopped'})
            done = True
            return
        proc.stdin.write(json.dumps(data) + '\n')
        proc.stdin.flush()
        current = json.loads(proc.stdout.readline())
        self.respond(current)
        done = current.get('success', False) or current.get('event') == 'finished'

server = HTTPServer(('127.0.0.1', args.port), Handler)
server.timeout = 1
print(json.dumps({'endpoint': f'http://127.0.0.1:{args.port}', 'observation': current}), flush=True)
try:
    while not done and proc.poll() is None:
        server.handle_request()
finally:
    server.server_close()
    proc.stdin.close()
    for line in proc.stdout:
        print(line, end='', flush=True)
    proc.wait()
