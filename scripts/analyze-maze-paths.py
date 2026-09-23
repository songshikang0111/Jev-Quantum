#!/usr/bin/env python3
"""Offline trajectory audit and sensitivity to spatial-v2's unspecified final ties.
Full-map shortest distance is an evaluation reference, never a player input.
"""
import argparse
from collections import Counter, deque
from itertools import permutations
import json

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('report')
args = parser.parse_args()
r = json.load(open(args.report))
m = r['maze']; w = m['width']; h = m['height']
start = tuple(m['start']); goal = tuple(m['exit'])
names = ['UP', 'RIGHT', 'DOWN', 'LEFT']
def neighbors(p):
    x,y=p
    for d,(dx,dy) in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
        if not m['cells'][y*w+x]['nesw'[d]]:
            yield d,(x+dx,y+dy)
def edge(a,b): return tuple(sorted((a,b)))
class Memory:
    def __init__(self):
        self.visits=Counter(); self.edges=Counter(); self.known={}
    def observe(self,p):
        self.visits[p]+=1
        self.known[p]=list(neighbors(p)) # sensor: current cell only
    def key(self,p,q):
        frontier = any(z not in self.known for _,z in self.known.get(q,[]))
        return (self.edges[edge(p,q)], q in self.known, not frontier, self.visits[q])
    def move(self,p,q): self.edges[edge(p,q)]+=1

dist={goal:0}; queue=deque([goal])
while queue:
    p=queue.popleft()
    for _,q in neighbors(p):
        if q not in dist: dist[q]=dist[p]+1; queue.append(q)
results=[]
for target in r['targets']:
    memory=Memory(); p=start; seen={p}; repeats=0; stagnant=0; longest=0; violations=[]; violation_details=[]; ties=0
    steps=[s for s in target['trajectory']['steps'] if s.get('kind')!='pace_gap']
    for i,s in enumerate(steps,1):
        memory.observe(p)
        choices=memory.known[p]; keys={q:memory.key(p,q) for _,q in choices}
        best=min(keys.values()); q=(s['x'],s['y'])
        if q in keys and keys[q]!=best:
            violations.append(i)
            violation_details.append(dict(step=i,position=p,selected=q,selected_key=keys[q],best_key=best))
        if sum(k==best for k in keys.values())>1: ties+=1
        if q in seen: repeats+=1; stagnant+=1
        else: seen.add(q); stagnant=0
        longest=max(longest,stagnant)
        memory.move(p,q); p=q
    results.append(dict(name=target['name'], steps=len(steps), reached=p==goal,
        unique_cells=len(seen), arrivals_at_seen_cells=repeats,longest_no_discovery=longest,
        max_edge_traversals=max(memory.edges.values(),default=0),
        traversals_beyond_second=sum(max(0,n-2) for n in memory.edges.values()),
        v2_priority_violation_steps=violations,v2_priority_violation_details=violation_details,v2_tie_decisions=ties))
runs=[]
for order in permutations(range(4)):
    memory=Memory(); p=start; n=0
    while p!=goal and n<10000:
        memory.observe(p)
        d,q=min(memory.known[p],key=lambda dq:(memory.key(p,dq[1]),order.index(dq[0])))
        memory.move(p,q); p=q; n+=1
    runs.append(dict(tie_order=[names[d] for d in order],steps=n,reached=p==goal))
print(json.dumps(dict(seed=m['seed'],oracle_shortest_steps=dist[start], trajectories=results,
    memory_rules_tie_permutations=runs),indent=2))
