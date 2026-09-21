use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use indexmap::IndexMap;
use jev_quantum_core::protocol::{Question, SystemOneRequest};
use jev_quantum_core::rng::{Entropy, Xoshiro256PlusPlus};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::record::{action_from_choice, action_name};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CellWalls {
    pub n: bool,
    pub e: bool,
    pub s: bool,
    pub w: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Maze {
    pub width: usize,
    pub height: usize,
    pub seed: u64,
    pub start: [usize; 2],
    pub exit: [usize; 2],
    pub cells: Vec<CellWalls>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    #[default]
    Decision,
    PaceGap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MazeStep {
    pub x: usize,
    pub y: usize,
    pub action: String,
    pub collision: bool,
    pub latency_ns: u64,
    #[serde(default)]
    pub throttle_ns: u64,
    #[serde(default)]
    pub kind: StepKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    #[default]
    Unspecified,
    Exit,
    MaxSteps,
    MaxRuntime,
    Interrupted,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Exit => "exit",
            Self::MaxSteps => "max_steps",
            Self::MaxRuntime => "max_runtime",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn aborts_remaining_targets(self) -> bool {
        matches!(self, Self::MaxRuntime | Self::Interrupted)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MazeTrajectory {
    pub steps: Vec<MazeStep>,
    pub success: bool,
    pub exit_step: Option<usize>,
    #[serde(default)]
    pub stop_reason: StopReason,
}

/// Hard caps checked off the local hot path except one atomic load and one `Instant`.
#[inline]
pub fn maze_cap_hit(cancelled: bool, now: Instant, deadline: Instant) -> Option<StopReason> {
    if cancelled {
        Some(StopReason::Interrupted)
    } else if now >= deadline {
        Some(StopReason::MaxRuntime)
    } else {
        None
    }
}

#[inline]
pub fn cancel_flag(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}

pub fn maze_deadline(max_runtime_secs: u64) -> Instant {
    if max_runtime_secs == 0 {
        Instant::now() + std::time::Duration::from_secs(100 * 365 * 24 * 60 * 60)
    } else {
        Instant::now() + std::time::Duration::from_secs(max_runtime_secs)
    }
}

impl Maze {
    pub fn braided(width: usize, height: usize, seed: u64) -> Self {
        let width = width.max(2);
        let height = height.max(2);
        let mut rng = Xoshiro256PlusPlus::from_seed(seed);
        let mut cells = vec![
            CellWalls {
                n: true,
                e: true,
                s: true,
                w: true,
            };
            width * height
        ];
        let mut visited = vec![false; width * height];
        carve(0, 0, width, height, &mut cells, &mut visited, &mut rng);
        braid(&mut cells, width, height, &mut rng);
        Self {
            width,
            height,
            seed,
            start: [0, 0],
            exit: [width - 1, height - 1],
            cells,
        }
    }

    fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    pub fn try_move(&self, x: usize, y: usize, action: u8) -> (usize, usize, bool) {
        let cell = self.cells[self.index(x, y)];
        let blocked = match action {
            0 => cell.n || y == 0,
            1 => cell.e || x + 1 >= self.width,
            2 => cell.s || y + 1 >= self.height,
            3 => cell.w || x == 0,
            _ => true,
        };
        if blocked {
            return (x, y, true);
        }
        match action {
            0 => (x, y - 1, false),
            1 => (x + 1, y, false),
            2 => (x, y + 1, false),
            3 => (x - 1, y, false),
            _ => (x, y, true),
        }
    }

    pub fn is_exit(&self, x: usize, y: usize) -> bool {
        [x, y] == self.exit
    }

    pub fn manhattan(&self, x: usize, y: usize) -> i32 {
        (self.exit[0] as i32 - x as i32).abs() + (self.exit[1] as i32 - y as i32).abs()
    }

    pub fn visit(&self, visits: &[u32], x: usize, y: usize) -> u32 {
        visits[self.index(x, y)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MazeContext {
    Features,
    Minimal,
}

impl MazeContext {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "features" | "feature" => Ok(Self::Features),
            "minimal" | "bare" => Ok(Self::Minimal),
            other => Err(format!(
                "unknown maze context '{other}' (use features|minimal)"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Features => "features",
            Self::Minimal => "minimal",
        }
    }
}

const DIRS: [(&str, u8); 4] = [("UP", 0), ("RIGHT", 1), ("DOWN", 2), ("LEFT", 3)];

pub fn situation_name(open_count: usize) -> &'static str {
    match open_count {
        0 => "trapped",
        1 => "dead_end",
        2 => "corridor",
        _ => "junction",
    }
}

fn carve(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cells: &mut [CellWalls],
    visited: &mut [bool],
    rng: &mut Xoshiro256PlusPlus,
) {
    visited[y * width + x] = true;
    let mut dirs = [0u8, 1, 2, 3];
    for i in (1..dirs.len()).rev() {
        let j = rng.bounded_usize(i + 1);
        dirs.swap(i, j);
    }
    for dir in dirs {
        let (nx, ny) = match dir {
            0 if y > 0 => (x, y - 1),
            1 if x + 1 < width => (x + 1, y),
            2 if y + 1 < height => (x, y + 1),
            3 if x > 0 => (x - 1, y),
            _ => continue,
        };
        if visited[ny * width + nx] {
            continue;
        }
        knock(cells, width, height, x, y, dir);
        carve(nx, ny, width, height, cells, visited, rng);
    }
}

fn braid(cells: &mut [CellWalls], width: usize, height: usize, rng: &mut Xoshiro256PlusPlus) {
    for y in 0..height {
        for x in 0..width {
            if rng.bounded_usize(4) != 0 {
                continue;
            }
            let dir = rng.bounded_usize(4) as u8;
            let (nx, ny) = match dir {
                0 if y > 0 => (x, y - 1),
                1 if x + 1 < width => (x + 1, y),
                2 if y + 1 < height => (x, y + 1),
                3 if x > 0 => (x - 1, y),
                _ => continue,
            };
            knock(cells, width, height, x, y, dir);
            let _ = (nx, ny);
        }
    }
}

fn knock(cells: &mut [CellWalls], width: usize, height: usize, x: usize, y: usize, dir: u8) {
    let i = y * width + x;
    match dir {
        0 if y > 0 => {
            cells[i].n = false;
            cells[i - width].s = false;
        }
        1 if x + 1 < width => {
            cells[i].e = false;
            cells[i + 1].w = false;
        }
        2 if y + 1 < height => {
            cells[i].s = false;
            cells[i + width].n = false;
        }
        3 if x > 0 => {
            cells[i].w = false;
            cells[i - 1].e = false;
        }
        _ => {}
    }
}

pub fn step_request(
    model: &str,
    maze: &Maze,
    x: usize,
    y: usize,
    visits: &[u32],
    last_move: Option<&str>,
    context: MazeContext,
) -> SystemOneRequest {
    match context {
        MazeContext::Minimal => minimal_request(model, maze, x, y),
        MazeContext::Features => feature_request(model, maze, x, y, visits, last_move),
    }
}

fn minimal_request(model: &str, maze: &Maze, x: usize, y: usize) -> SystemOneRequest {
    let mut criteria = IndexMap::new();
    for (name, _) in DIRS {
        criteria.insert(name.to_string(), Some(format!("Move {name}")));
    }
    choice_only(
        model,
        json!({
            "task": "maze",
            "position": [x, y],
            "exit": maze.exit,
            "size": [maze.width, maze.height],
        }),
        "Choose the next move through the maze.",
        criteria,
    )
}

fn feature_request(
    model: &str,
    maze: &Maze,
    x: usize,
    y: usize,
    visits: &[u32],
    last_move: Option<&str>,
) -> SystemOneRequest {
    let here = maze.manhattan(x, y);
    let east = maze.exit[0] as i32 - x as i32;
    let south = maze.exit[1] as i32 - y as i32;
    let mut neighbors = IndexMap::new();
    let mut criteria = IndexMap::new();

    for (name, dir) in DIRS {
        let (nx, ny, blocked) = maze.try_move(x, y, dir);
        let visited = if blocked {
            0
        } else {
            maze.visit(visits, nx, ny)
        };
        let closer = !blocked && maze.manhattan(nx, ny) < here;
        neighbors.insert(
            name,
            json!({
                "open": !blocked,
                "visited": visited,
                "closer_to_exit": closer,
            }),
        );
        if blocked {
            continue;
        }
        let closer_text = if closer {
            "reduces manhattan distance to the exit"
        } else {
            "does not reduce manhattan distance to the exit"
        };
        criteria.insert(
            name.to_string(),
            Some(format!(
                "Open neighbor. Visited {visited} time(s). This step {closer_text}."
            )),
        );
    }

    if criteria.is_empty() {
        for (name, _) in DIRS {
            criteria.insert(
                name.to_string(),
                Some("No open neighbor; every direction is a wall.".into()),
            );
        }
    }

    let open_count = neighbors
        .values()
        .filter(|v| v["open"].as_bool() == Some(true))
        .count();
    let situation = situation_name(open_count);

    let mut questions = IndexMap::new();
    questions.insert(
        "move".into(),
        Question::Choice {
            instructions: Some(feature_instructions(situation)),
            criteria,
        },
    );
    questions.insert(
        "progress_available".into(),
        Question::Noul {
            instructions: Some(
                "Is there at least one open neighbor that reduces manhattan distance to the exit?"
                    .into(),
            ),
            criteria: None,
        },
    );

    SystemOneRequest {
        model: model.to_string(),
        state: json!({
            "task": "Navigate a 4-connected grid maze. Walls block movement. Reaching the exit cell succeeds. Do not step into a wall.",
            "position": [x, y],
            "exit": maze.exit,
            "size": [maze.width, maze.height],
            "goal": { "east": east, "south": south, "manhattan": here },
            "last_move": last_move,
            "situation": situation,
            "visited_count_here": maze.visit(visits, x, y),
            "neighbors": neighbors,
        }),
        questions,
    }
}

fn feature_instructions(situation: &str) -> String {
    format!(
        "Pick one legal open direction. Prefer an open neighbor that reduces manhattan distance to the exit. If every closer cell is blocked or heavily visited, pick the least-visited open cell. At a {situation}, never choose a wall."
    )
}

fn choice_only(
    model: &str,
    state: serde_json::Value,
    instructions: &str,
    criteria: IndexMap<String, Option<String>>,
) -> SystemOneRequest {
    let mut questions = IndexMap::new();
    questions.insert(
        "move".into(),
        Question::Choice {
            instructions: Some(instructions.to_string()),
            criteria,
        },
    );
    SystemOneRequest {
        model: model.to_string(),
        state,
        questions,
    }
}

pub fn decode_move(response: &jev_quantum_core::SystemOneResponse) -> u8 {
    response
        .answers
        .get("move")
        .and_then(|ans| ans.choice_label())
        .map(action_from_choice)
        .unwrap_or(255)
}

pub fn apply_choice(
    maze: &Maze,
    x: usize,
    y: usize,
    action: u8,
    latency_ns: u64,
    throttle_ns: u64,
) -> MazeStep {
    let (nx, ny, collision) = maze.try_move(x, y, action);
    MazeStep {
        x: nx,
        y: ny,
        action: action_name(action).to_string(),
        collision,
        latency_ns,
        throttle_ns,
        kind: StepKind::Decision,
    }
}

pub fn pace_gap_step(x: usize, y: usize, throttle_ns: u64) -> MazeStep {
    MazeStep {
        x,
        y,
        action: "PACE".to_string(),
        collision: false,
        latency_ns: 0,
        throttle_ns,
        kind: StepKind::PaceGap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    #[test]
    fn generated_maze_is_carved() {
        let maze = Maze::braided(12, 12, 3);
        assert_eq!(maze.cells.len(), 144);
        let open = maze
            .cells
            .iter()
            .filter(|c| !c.n || !c.e || !c.s || !c.w)
            .count();
        assert!(open > 20);
    }

    #[test]
    fn feature_request_keeps_only_open_moves() {
        let maze = Maze::braided(8, 8, 7);
        let mut visits = vec![0u32; 64];
        visits[0] = 1;
        let req = step_request(
            "jev-quantum-latest",
            &maze,
            0,
            0,
            &visits,
            None,
            MazeContext::Features,
        );
        let Question::Choice { criteria, .. } = &req.questions["move"] else {
            panic!("expected choice");
        };
        assert!(!criteria.is_empty());
        for name in criteria.keys() {
            let dir = action_from_choice(name);
            let (_, _, blocked) = maze.try_move(0, 0, dir);
            assert!(!blocked, "{name} should be open");
        }
        assert!(req.questions.contains_key("progress_available"));
        assert!(req.state["situation"].as_str().is_some());
        assert!(req.state["neighbors"]["RIGHT"].is_object());
    }

    #[test]
    fn situation_labels_match_open_count() {
        assert_eq!(situation_name(0), "trapped");
        assert_eq!(situation_name(1), "dead_end");
        assert_eq!(situation_name(2), "corridor");
        assert_eq!(situation_name(3), "junction");
    }

    #[test]
    fn pace_gap_is_not_a_decision() {
        let gap = pace_gap_step(2, 3, 12_000_000);
        assert_eq!(gap.kind, StepKind::PaceGap);
        assert_eq!(gap.action, "PACE");
        assert_eq!(gap.latency_ns, 0);
        assert_eq!(gap.throttle_ns, 12_000_000);
    }

    #[test]
    fn closer_flag_follows_manhattan() {
        let maze = Maze::braided(6, 6, 1);
        let here = maze.manhattan(0, 0);
        assert!(here > 0);
        let (_, _, blocked) = maze.try_move(0, 0, 1);
        if !blocked {
            assert!(maze.manhattan(1, 0) <= here);
        }
    }

    #[test]
    fn cap_priority_is_interrupt_then_runtime() {
        let now = Instant::now();
        let deadline = now + std::time::Duration::from_secs(1);
        assert_eq!(
            maze_cap_hit(true, now, deadline),
            Some(StopReason::Interrupted)
        );
        assert_eq!(
            maze_cap_hit(false, deadline, deadline),
            Some(StopReason::MaxRuntime)
        );
        assert_eq!(maze_cap_hit(false, now, deadline), None);
    }

    #[test]
    fn old_trajectory_json_defaults_stop_reason() {
        let parsed: MazeTrajectory = serde_json::from_value(json!({
            "steps": [],
            "success": false,
            "exit_step": null
        }))
        .unwrap();
        assert_eq!(parsed.stop_reason, StopReason::Unspecified);
        assert!(!parsed.stop_reason.aborts_remaining_targets());
        assert!(StopReason::MaxRuntime.aborts_remaining_targets());
        assert!(StopReason::Interrupted.aborts_remaining_targets());
        assert!(!StopReason::MaxSteps.aborts_remaining_targets());
        assert!(!StopReason::Exit.aborts_remaining_targets());
    }
}
