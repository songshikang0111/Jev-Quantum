//! Historical v1 prompt and state shape, with an untruncated transition history.
//! Only `observe` receives the current cell's walls; the map limit remains 64.
use crate::maze::CellWalls;
use crate::record::action_name;
use jev_quantum_core::protocol::{Question, SystemOneRequest};
use serde_json::json;
use std::collections::VecDeque;

pub struct TrajectoryMemory {
    width: usize,
    height: usize,
    walls: Vec<[Option<bool>; 4]>,
    observed: Vec<bool>,
    visits: Vec<u32>,
    edges: Vec<[u32; 4]>,
    recent: VecDeque<serde_json::Value>,
    decisions: usize,
}

impl TrajectoryMemory {
    /// Remove derived goal-proximity cues, retaining the goal coordinates and prompt.
    pub fn remove_distance_guidance(req: &mut SystemOneRequest) {
        if let Some(state) = req.state.as_object_mut() {
            state.remove("goal");
            if let Some(neighbors) = state.get_mut("neighbors").and_then(|v| v.as_object_mut()) {
                for neighbor in neighbors.values_mut() {
                    if let Some(info) = neighbor.as_object_mut() {
                        info.remove("closer_to_exit");
                    }
                }
            }
        }
        req.questions.shift_remove("progress_available");
        if let Some(Question::Choice { criteria, .. }) = req.questions.get_mut("move") {
            for description in criteria.values_mut().flatten() {
                *description = description
                    .replace(" This step reduces manhattan distance to the exit.", "")
                    .replace(
                        " This step does not reduce manhattan distance to the exit.",
                        "",
                    );
            }
        }
    }

    /// Prompt-only ablation: preserve the entire state, criteria and other questions.
    pub fn use_free_strategy_prompt(req: &mut SystemOneRequest) {
        if let Some(Question::Choice { instructions, .. }) = req.questions.get_mut("move") {
            *instructions = Some("Choose a legal move using the trajectory and explored map. Avoid repeating loops and repeatedly traversing the same edge. Prefer unexplored branches; backtrack through explored cells when necessary to reach an unexplored branch. Choose the strategy you judge most effective for reaching the exit based on the complete history and known map. You are not required to reduce distance to the exit on each move; moving away from it is allowed whenever you judge that useful. Use prior transitions to recognize failed detours. Never choose a wall.".into());
        }
    }

    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            walls: vec![[None; 4]; width * height],
            observed: vec![false; width * height],
            visits: vec![0; width * height],
            edges: vec![[0; 4]; width * height],
            recent: VecDeque::new(),
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
        self.observed[i] = true;
        self.visits[i] = self.visits[i].saturating_add(1);
        for (d, wall) in [walls.n, walls.e, walls.s, walls.w].into_iter().enumerate() {
            self.walls[i][d] = Some(wall);
            if let Some(j) = self.neighbor(i, d) {
                self.walls[j][(d + 2) % 4] = Some(wall);
            }
        }
    }

    pub fn record(&mut self, from: [usize; 2], to: [usize; 2], action: u8, collision: bool) {
        if action < 4 {
            let i = from[1] * self.width + from[0];
            self.edges[i][action as usize] = self.edges[i][action as usize].saturating_add(1);
        }
        self.decisions += 1;
        self.recent.push_back(
            json!({"from": from, "action": action_name(action), "to": to, "collision": collision}),
        );
        // Long-history variant retains every transition up to the run step cap.
    }

    pub fn enrich(&self, req: &mut SystemOneRequest, x: usize, y: usize) {
        let i = y * self.width + x;
        // Bounded detail for arbitrary maze sizes; counts and local edges retain all history.
        let mut cells: Vec<usize> = (0..self.walls.len())
            .filter(|&j| self.observed[j])
            .collect();
        cells.sort_by_key(|&j| {
            (
                j != i,
                (j % self.width).abs_diff(x) + (j / self.width).abs_diff(y),
                j,
            )
        });
        let total = cells.len();
        cells.truncate(64);
        let map: Vec<_> = cells
            .into_iter()
            .map(|j| {
                json!({
                    "position": [j % self.width, j / self.width], "visits": self.visits[j],
                    "walls_URDL": self.walls[j], "departures_URDL": self.edges[j],
                })
            })
            .collect();
        req.state["memory"] = json!({
            "total_decisions": self.decisions, "observed_cells": total,
            "recent_transitions": self.recent, "recent_limit": null,
            "known_map_nearby": map, "map_limit": 64, "map_truncated": total > 64,
            "departures_here_URDL": self.edges[i],
            "note": "Only observed walls are known. Counts summarize the entire walk; recent transitions are chronological. URDL means UP RIGHT DOWN LEFT."
        });
        if let Some(Question::Choice {
            instructions,
            criteria,
        }) = req.questions.get_mut("move")
        {
            *instructions = Some("Choose a legal move using the trajectory and explored map. Avoid repeating loops and repeatedly traversing the same edge. Prefer unexplored branches; backtrack through explored cells when necessary to reach an unexplored branch. Manhattan distance is only a tie-breaker: moving away from the exit can be necessary. Use prior transitions to recognize failed detours. Never choose a wall.".into());
            for (d, count) in self.edges[i].iter().enumerate() {
                if let Some(description) = criteria.get_mut(action_name(d as u8)) {
                    let old = description.take().unwrap_or_default();
                    *description = Some(format!(
                        "{old} Previously departed this cell in this direction {count} time(s)."
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maze::{step_request, Maze, MazeContext};

    #[test]
    fn removing_distance_preserves_memory_prompt_and_legal_moves() {
        let maze = Maze::braided(10, 10, 20260935);
        let mut nav = TrajectoryMemory::new(10, 10);
        nav.observe(0, 0, maze.cells[0]);
        for _ in 0..100 {
            nav.record([0, 0], [0, 0], 0, true);
        }
        let mut req = step_request("jev", &maze, 0, 0, &[0; 100], None, MazeContext::Features);
        nav.enrich(&mut req, 0, 0);
        TrajectoryMemory::use_free_strategy_prompt(&mut req);
        let mut expected = serde_json::to_value(&req).unwrap();
        expected["state"].as_object_mut().unwrap().remove("goal");
        for v in expected["state"]["neighbors"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            v.as_object_mut().unwrap().remove("closer_to_exit");
        }
        expected["questions"]
            .as_object_mut()
            .unwrap()
            .remove("progress_available");
        for v in expected["questions"]["move"]["criteria"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            let old = v.as_str().unwrap();
            let sentences: Vec<_> = old
                .split(". ")
                .filter(|s| !s.starts_with("This step "))
                .collect();
            *v = json!(sentences.join(". "));
        }
        TrajectoryMemory::remove_distance_guidance(&mut req);
        assert_eq!(serde_json::to_value(&req).unwrap(), expected);
        assert_eq!(
            req.state["memory"]["recent_transitions"]
                .as_array()
                .unwrap()
                .len(),
            100
        );
        assert_eq!(req.state["exit"], json!(maze.exit));
    }

    #[test]
    fn free_strategy_changes_only_move_instructions() {
        let maze = Maze::braided(10, 10, 20260935);
        let mut nav = TrajectoryMemory::new(10, 10);
        nav.observe(0, 0, maze.cells[0]);
        for _ in 0..100 {
            nav.record([0, 0], [0, 0], 0, true);
        }
        let mut req = step_request("jev", &maze, 0, 0, &[0; 100], None, MazeContext::Features);
        nav.enrich(&mut req, 0, 0);
        let before = serde_json::to_value(&req).unwrap();
        TrajectoryMemory::use_free_strategy_prompt(&mut req);
        let mut after = serde_json::to_value(&req).unwrap();
        assert_ne!(
            after["questions"]["move"]["instructions"],
            before["questions"]["move"]["instructions"]
        );
        after["questions"]["move"]["instructions"] =
            before["questions"]["move"]["instructions"].clone();
        assert_eq!(before, after);
    }

    #[test]
    fn full_history_retains_old_steps_without_revealing_unobserved_cells() {
        let maze = Maze::braided(10, 10, 7);
        let mut nav = TrajectoryMemory::new(10, 10);
        nav.observe(0, 0, maze.cells[0]);
        for _ in 0..40 {
            nav.record([0, 0], [0, 0], 0, true);
        }
        let mut req = step_request(
            "jev",
            &maze,
            0,
            0,
            &vec![0; 100],
            None,
            MazeContext::Features,
        );
        nav.enrich(&mut req, 0, 0);
        assert_eq!(
            req.state["memory"]["recent_transitions"]
                .as_array()
                .unwrap()
                .len(),
            40
        );
        assert_eq!(
            req.state["memory"]["known_map_nearby"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(req.state["memory"]["departures_here_URDL"][0], 40);
        assert_eq!(req.state["memory"]["total_decisions"], 40);
        assert!(req.state["memory"]["recent_limit"].is_null());
        assert!(req.state.get("flood_fill").is_none());
    }

    #[test]
    fn history_is_chronological_and_map_limit_is_unchanged() {
        let maze = Maze::braided(10, 10, 7);
        let mut nav = TrajectoryMemory::new(10, 10);
        for i in 0..100 {
            nav.observe(i % 10, i / 10, maze.cells[i]);
            nav.record([i % 10, i / 10], [i % 10, i / 10], 0, true);
        }
        let mut req = step_request("jev", &maze, 9, 9, &[0; 100], None, MazeContext::Features);
        let original_position = req.state["position"].clone();
        nav.enrich(&mut req, 9, 9);
        let memory = &req.state["memory"];
        assert_eq!(memory["recent_transitions"].as_array().unwrap().len(), 100);
        assert_eq!(memory["recent_transitions"][0]["from"], json!([0, 0]));
        assert_eq!(memory["recent_transitions"][99]["from"], json!([9, 9]));
        assert_eq!(memory["known_map_nearby"].as_array().unwrap().len(), 64);
        assert_eq!(memory["map_truncated"], true);
        assert_eq!(req.state["position"], original_position);
    }
}
