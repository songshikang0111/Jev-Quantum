#!/usr/bin/env python3
"""Audit a Luna rollout and attach separate session/call/token metrics to its report."""
import argparse
from datetime import datetime
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--rollout', type=Path, required=True)
parser.add_argument('--report', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
rows = [json.loads(line) for line in args.rollout.read_text().splitlines()]
starts = {}
durations = []
usages = []
models = set()
tools = []
unknown = []

allowed = set()
for action in ('UP', 'RIGHT', 'DOWN', 'LEFT'):
    cmd = "curl -q -sS --max-time 10 -X POST http://127.0.0.1:3187/move -H 'Content-Type: application/json' --data '{\"action\":\"" + action + "\"}'"
    allowed.add('text((await tools.exec_command({cmd:' + json.dumps(cmd) + ',max_output_tokens:700})).output);')
    allowed.add("text(await tools.write_stdin({session_id:48810,chars:'{\"action\":\"" + action + "\"}\\n',yield_time_ms:1,max_output_tokens:700}));")

for row in rows:
    p = row.get('payload', {})
    kind = p.get('type')
    if row['type'] == 'turn_context':
        models.add(p.get('model'))
    if row['type'] == 'token_usage_record':
        usages.append(p['usage'])
    if kind == 'task_started':
        starts[p['turn_id']] = datetime.fromisoformat(row['timestamp'].replace('Z', '+00:00'))
    if kind == 'task_complete':
        start = starts.get(p.get('turn_id'))
        if start:
            end = datetime.fromisoformat(row['timestamp'].replace('Z', '+00:00'))
            durations.append((end - start).total_seconds())
    if kind in ('function_call', 'custom_tool_call'):
        tools.append({'name': p.get('name'), 'input': p.get('input', p.get('arguments', ''))})
        if p.get('name') != 'exec' or p.get('input', '').strip() not in allowed:
            unknown.append(tools[-1])

report = json.loads(args.report.read_text())
target = next(t for t in report['targets'] if t['name'] == 'luna-session')
metrics = report['config']['session_metrics']
metrics.pop('model_turns', None)
metrics.update({
    'decision_turns': len(target['trajectory']['steps']),
    'subagent_turns': len(starts),
    'completed_subagent_turns': len(durations),
    'model_calls': len(usages),
    'agent_active_wall_seconds': sum(durations),
    'agent_active_definition': 'sum of subagent task-start to task-complete intervals, includes its game tool calls; excludes parent dispatch gaps; not pure inference time',
    'token_usage': {key: sum(u.get(key, 0) for u in usages) for key in ('input_tokens', 'cached_input_tokens', 'output_tokens', 'reasoning_output_tokens', 'total_tokens')},
    'models_observed': sorted(models),
    'tool_calls': len(tools),
    'unexpected_tool_calls': unknown,
    'tool_allowlist_passed': not unknown,
    'controller_notes': 'First 28 actions were parent-relayed; the 600s broker deadline expired during dispatch/questions. Same player session and exact game state then resumed through a game-only HTTP endpoint. Two failed terminal-handle calls made no game moves. Total broker wall time includes the first segment; no full maze or seed was supplied to the player.',
    'rollout_id': args.rollout.stem,
})
metrics['token_usage']['uncached_input_tokens'] = metrics['token_usage']['input_tokens'] - metrics['token_usage']['cached_input_tokens']
args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
args.output.write_text(json.dumps(metrics, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(metrics, ensure_ascii=False, indent=2))
if unknown:
    raise SystemExit('Audit requires review: unexpected tool call found')
