//! Online navigation: only `observe` receives the walls of the current cell.
use crate::maze::CellWalls;
use crate::record::action_name;
use jev_quantum_core::protocol::{Question, SystemOneRequest};
use serde_json::json;
use std::collections::VecDeque;

pub struct Navigation {
    width: usize,
    height: usize,
    walls: Vec<[Option<bool>; 4]>,
    observed: Vec<bool>,
    visits: Vec<u32>,
    edges: Vec<[u32; 4]>,
    last_discovery: usize,
    observed_count: usize,
    decisions: usize,
}

impl Navigation {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            walls: vec![[None; 4]; width * height],
            observed: vec![false; width * height],
            visits: vec![0; width * height],
            edges: vec![[0; 4]; width * height],
            last_discovery: 0,
            observed_count: 0,
            decisions: 0,
        }
    }

    fn neighbor(&self, i: usize, d: usize) -> Option<usize> {
        let (x, y) = (i % self.width, i / self.width);
        match d {
            0 if y > 0 => Some(i - self.width),
            1 if x + 1 < self.width => Some(i + 1),
            2 if y + 1 < self.height => Some(i + self.width),
            3 if x > 0 => Some(i - 1),
            _ => None,
        }
    }

    pub fn observe(&mut self, x: usize, y: usize, walls: CellWalls) {
        let i = y * self.width + x;
        if !self.observed[i] {
            self.last_discovery = self.decisions;
            self.observed_count += 1;
        }
        self.observed[i] = true;
        self.visits[i] = self.visits[i].saturating_add(1);
        for (d, wall) in [walls.n, walls.e, walls.s, walls.w].into_iter().enumerate() {
            self.walls[i][d] = Some(wall);
            if let Some(j) = self.neighbor(i, d) {
                self.walls[j][(d + 2) % 4] = Some(wall);
            }
        }
    }

    /// Optimistic flood fill: unknown edges remain open until a sensor disproves them.
    /// Descending the field either reaches the goal or discovers a wall and replans.
    fn distances(&self, exit: [usize; 2]) -> Vec<usize> {
        let mut distance = vec![usize::MAX; self.walls.len()];
        let goal = exit[1] * self.width + exit[0];
        distance[goal] = 0;
        let mut queue = VecDeque::from([goal]);
        while let Some(i) = queue.pop_front() {
            for d in 0..4 {
                if self.walls[i][d] == Some(true) {
                    continue;
                }
                if let Some(j) = self.neighbor(i, d) {
                    if distance[j] == usize::MAX {
                        distance[j] = distance[i] + 1;
                        queue.push_back(j);
                    }
                }
            }
        }
        distance
    }

    pub fn choose(&self, x: usize, y: usize, exit: [usize; 2]) -> Option<u8> {
        let distance = self.distances(exit);
        let i = y * self.width + x;
        (0..4)
            .filter_map(|d| {
                let j = self.neighbor(i, d)?;
                (self.walls[i][d] == Some(false) && distance[j] != usize::MAX).then_some((
                    (
                        distance[j],
                        self.observed[j],
                        self.visits[j],
                        self.edges[i][d],
                        d,
                    ),
                    d as u8,
                ))
            })
            .min_by_key(|(key, _)| *key)
            .map(|(_, d)| d)
    }

    pub fn record(&mut self, from: [usize; 2], to: [usize; 2], action: u8, collision: bool) {
        if action < 4 {
            let i = from[1] * self.width + from[0];
            self.edges[i][action as usize] = self.edges[i][action as usize].saturating_add(1);
        }
        self.decisions += 1;
        let _ = (to, collision); // Only spatial counts are retained, never a trajectory.
    }

    pub fn observed_count(&self) -> usize {
        self.observed_count
    }
    pub fn stagnant_steps(&self) -> usize {
        self.decisions.saturating_sub(self.last_discovery)
    }

    fn unexplored_exits(&self, i: usize) -> usize {
        (0..4)
            .filter(|&d| {
                self.walls[i][d] == Some(false)
                    && self.neighbor(i, d).is_some_and(|j| !self.observed[j])
            })
            .count()
    }

    fn traversals(&self, i: usize, d: usize) -> u32 {
        self.edges[i][d].saturating_add(
            self.neighbor(i, d)
                .map(|j| self.edges[j][(d + 2) % 4])
                .unwrap_or(0),
        )
    }

    /// A fixed-size spatial projection: at most 24 observed cells plus four moves.
    /// Flood-fill computes global distances locally; Jev receives the local gradient.
    /// All legal choices remain available and the remote answer is never overridden.
    pub fn enrich(
        &self,
        req: &mut SystemOneRequest,
        x: usize,
        y: usize,
        exit: [usize; 2],
        flood: bool,
    ) {
        let i = y * self.width + x;
        let distance = flood.then(|| self.distances(exit));
        let finite = |d: usize| (d != usize::MAX).then_some(d);
        let mut cells: Vec<_> = (0..self.walls.len())
            .filter(|&j| self.observed[j])
            .collect();
        cells.sort_by_key(|&j| {
            (
                j != i,
                (j % self.width).abs_diff(x) + (j / self.width).abs_diff(y),
                j,
            )
        });
        cells.truncate(24);
        let map: Vec<_> = cells
            .into_iter()
            .map(|j| {
                let mask = (0..4).fold(0u8, |mask, d| {
                    mask | (u8::from(self.walls[j][d] == Some(true)) << d)
                });
                json!([
                    j % self.width,
                    j / self.width,
                    mask,
                    self.visits[j],
                    self.edges[j]
                ])
            })
            .collect();
        let mut moves = serde_json::Map::new();
        let mut criteria = indexmap::IndexMap::new();
        for d in 0..4 {
            if self.walls[i][d] != Some(false) {
                continue;
            }
            let Some(j) = self.neighbor(i, d) else {
                continue;
            };
            let traversals = self.traversals(i, d);
            let frontier = self.unexplored_exits(j);
            let mut info = json!({
                "position": [j % self.width, j / self.width],
                "explored": self.observed[j], "visits": self.visits[j],
                "edge_traversals": traversals, "departures": self.edges[i][d],
                "unexplored_exits": if self.observed[j] { Some(frontier) } else { None },
            });
            let description = if let Some(field) = &distance {
                info["distance_to_exit"] = json!(finite(field[j]));
                info["downhill"] = json!(field[j] < field[i]);
                format!("BFS distance={:?}; downhill={}; explored={}; visits={}; traversals={}. Minimize BFS distance first, then prefer unexplored and less visited cells.", finite(field[j]), field[j] < field[i], self.observed[j], self.visits[j], traversals)
            } else {
                format!("Edge traversals={traversals}; explored={}; visits={}; unexplored exits={:?}. Prefer the least-traversed edge; on ties prefer unexplored cells or unexplored branches.", self.observed[j], self.visits[j], if self.observed[j] { Some(frontier) } else { None })
            };
            moves.insert(action_name(d as u8).into(), info);
            criteria.insert(action_name(d as u8).into(), Some(description));
        }
        req.state = json!({
            "task": "Navigate an unknown maze to the exit using the spatial memory below. All listed moves are legal.",
            "position": [x,y], "exit": exit, "size": [self.width,self.height],
            "memory": {
                "version": if flood { "flood-v1" } else { "spatial-v2" },
                "observed_cells": self.observed_count,
                "steps_without_discovery": self.stagnant_steps(),
                "stagnating": self.stagnant_steps() >= 8,
                "known_cells": map, "cell_limit": 24, "map_truncated": self.observed_count > 24,
                "cell_format": "[x,y,wall_bits,visits,departures_URDL]; wall bits UP=1 RIGHT=2 DOWN=4 LEFT=8. Only observed cells are listed. The complete observed wall map is retained locally.",
            },
            "moves": moves,
        });
        let instructions = if let Some(field) = distance {
            req.state["flood_fill"] = json!({
                "distance_here": finite(field[i]),
                "unknown_edges": "optimistically open; recomputed from the exit after every local wall observation",
                "rule": "Pick the legal neighbor with the smallest finite distance_to_exit. Prefer downhill=true. Ignore Manhattan distance and do not override BFS to avoid a visited cell. Among equal distances prefer unexplored, then lower visits, then fewer edge traversals."
            });
            "Follow the flood_fill rule exactly. Choose the minimum finite BFS distance among moves; break equal-distance ties by unexplored first, then fewer visits, then fewer traversals. A visited cell may be required to detour around walls. Return the direction label."
        } else {
            "Use spatial memory. Choose the legal edge with the LOWEST edge_traversals to stop repeating loops. Among equal counts prefer an unexplored neighbor, then one with unexplored_exits, then lower visits. Never prefer closeness to the exit over exploration. Backtracking is allowed when required; do not repeatedly follow a highly traversed edge while another is less traversed. Return the direction label."
        };
        req.questions.clear();
        req.questions.insert(
            "move".into(),
            Question::Choice {
                instructions: Some(instructions.into()),
                criteria,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maze::{step_request, Maze, MazeContext};

    #[test]
    fn online_solver_reaches_exit_across_seeds_and_rectangles() {
        for (w, h) in [
            (1, 1),
            (1, 19),
            (19, 1),
            (2, 2),
            (3, 17),
            (17, 3),
            (10, 10),
            (32, 24),
        ] {
            for seed in (0..30).chain([20260935, u64::MAX]) {
                let maze = Maze::braided(w, h, seed);
                let mut nav = Navigation::new(w, h);
                let (mut x, mut y) = (0, 0);
                for _ in 0..(w * h * w * h * 4) {
                    if maze.is_exit(x, y) {
                        break;
                    }
                    nav.observe(x, y, maze.cells[y * w + x]);
                    let d = nav.choose(x, y, maze.exit).expect("connected maze");
                    let (nx, ny, collision) = maze.try_move(x, y, d);
                    assert!(!collision);
                    nav.record([x, y], [nx, ny], d, collision);
                    (x, y) = (nx, ny);
                }
                assert!(maze.is_exit(x, y), "{w}x{h} seed={seed}");
            }
        }
    }

    #[test]
    fn spatial_context_is_bounded_and_has_no_trajectory() {
        let mut nav = Navigation::new(100, 100);
        let open = CellWalls {
            n: false,
            e: false,
            s: false,
            w: false,
        };
        for y in 0..100 {
            for x in 0..100 {
                nav.observe(x, y, open);
            }
        }
        for _ in 0..100_000 {
            nav.record([0, 0], [1, 0], 1, false);
        }
        let maze = Maze::braided(2, 2, 7);
        for flood in [false, true] {
            let mut req = step_request(
                "jev",
                &maze,
                0,
                0,
                &[1, 0, 0, 0],
                None,
                MazeContext::Features,
            );
            nav.enrich(&mut req, 0, 0, [99, 99], flood);
            assert_eq!(
                req.state["memory"]["known_cells"].as_array().unwrap().len(),
                24
            );
            assert_eq!(req.state["memory"]["map_truncated"], true);
            assert!(req.state["memory"].get("recent_transitions").is_none());
            assert!(serde_json::to_vec(&req).unwrap().len() < 12_000);
        }
    }

    #[test]
    fn flood_payload_uses_only_observed_walls_and_preserves_all_legal_choices() {
        let maze = Maze::braided(10, 10, 20260935);
        let mut nav = Navigation::new(10, 10);
        let mut x = 0;
        let mut y = 0;
        for _ in 0..100 {
            if maze.is_exit(x, y) {
                break;
            }
            nav.observe(x, y, maze.cells[y * 10 + x]);
            let mut req = step_request(
                "jev",
                &maze,
                x,
                y,
                &vec![0; 100],
                None,
                MazeContext::Features,
            );
            nav.enrich(&mut req, x, y, maze.exit, true);
            let Question::Choice { criteria, .. } = &req.questions["move"] else {
                panic!()
            };
            let legal = (0..4).filter(|&d| !maze.try_move(x, y, d).2).count();
            assert_eq!(criteria.len(), legal);
            assert_eq!(req.state["memory"]["observed_cells"], nav.observed_count());
            for row in req.state["memory"]["known_cells"].as_array().unwrap() {
                assert!(
                    nav.observed[row[1].as_u64().unwrap() as usize * 10
                        + row[0].as_u64().unwrap() as usize]
                );
            }
            let dir = nav.choose(x, y, maze.exit).unwrap();
            assert_eq!(req.state["moves"][action_name(dir)]["downhill"], true);
            let (nx, ny, collision) = maze.try_move(x, y, dir);
            nav.record([x, y], [nx, ny], dir, collision);
            (x, y) = (nx, ny);
        }
        assert!(maze.is_exit(x, y));
    }
}
