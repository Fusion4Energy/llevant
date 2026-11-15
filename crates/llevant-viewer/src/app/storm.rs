//! The edit storm (`llevant-viewer <model> --storm`): a scripted run
//! through the panels and the edits people make most (hide and show,
//! isolate, levels, colouring, selection, typing in the palette, dragging a
//! field's opacity), timing the UI thread's own work on each frame. Each
//! step prints its frames' median, 95th percentile and maximum; the rule it
//! checks is that no frame's own work comes near a frame at 60 Hz
//! (16.7 ms), whatever the model.

use std::time::Duration;

use llevant_scene::query::Query;

use crate::app::frame::Panel;
use crate::command::{Command, Edit};
use crate::document::Document;
use crate::selection::SelectMode;
use crate::state::Colouring;

/// What the storm does on one frame.
#[derive(Default)]
pub struct Frame {
    pub commands: Vec<Command>,
    /// Open the palette with this text (`Some("")` closes it).
    pub palette: Option<String>,
    /// A rule in the Find panel's box.
    pub find_rule: Option<String>,
}

struct Step {
    name: String,
    frames: Vec<Frame>,
}

/// Frames a step goes on for at least once its own are done, and at most
/// while the views catch up.
const SETTLE: usize = 10;
const SETTLE_MAX: usize = 2000;

pub struct Storm {
    steps: Vec<Step>,
    at: usize,
    /// Frames into the current step.
    frame: usize,
    /// Own work (ms) of each frame of the current step, and of all.
    own: Vec<f64>,
    all: Vec<f64>,
    started: bool,
    pub done: bool,
}

impl Storm {
    pub fn new() -> Storm {
        Storm {
            steps: Vec::new(),
            at: 0,
            frame: 0,
            own: Vec::new(),
            all: Vec::new(),
            started: false,
            done: false,
        }
    }

    /// What to do on this frame (before the commands of the frame run).
    pub fn step(&mut self, d: &Document) -> Frame {
        if !self.started {
            // Start once the model is shown and its cards (and labels) read.
            if d.deriving() || d.view3d.busy() || d.source.is_none() {
                return Frame::default();
            }
            self.steps = script(d);
            self.started = true;
        }
        let Some(step) = self.steps.get_mut(self.at) else {
            return Frame::default();
        };
        let k = self.frame;
        self.frame += 1;
        if k < step.frames.len() {
            return std::mem::take(&mut step.frames[k]);
        }
        Frame::default()
    }

    /// After the frame: how long the UI thread worked on it.
    pub fn record(&mut self, own: Duration, d: &Document) {
        if !self.started || self.done {
            return;
        }
        let ms = 1e3 * own.as_secs_f64();
        self.own.push(ms);
        self.all.push(ms);
        let step = &self.steps[self.at];
        let past = self.frame.saturating_sub(step.frames.len());
        let settled = past >= SETTLE && !d.deriving() && d.field_loading().is_none();
        if settled || past >= SETTLE_MAX {
            eprintln!(
                "{:<28} {}{}",
                step.name,
                summary(&self.own),
                if settled { "" } else { " (did not settle)" }
            );
            self.own.clear();
            self.frame = 0;
            self.at += 1;
            if self.at == self.steps.len() {
                eprintln!("{:<28} {}", "every frame", summary(&self.all));
                self.done = true;
            }
        }
    }
}

/// Frames, and their own work's median, 95th percentile and maximum.
fn summary(ms: &[f64]) -> String {
    let mut v = ms.to_vec();
    v.sort_by(f64::total_cmp);
    let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
    format!(
        "{:>4} frames, own work median {:5.2} ms, 95 % {:5.2} ms, max {:6.2} ms",
        v.len(),
        q(0.5),
        q(0.95),
        q(1.0)
    )
}

fn one(name: &str, commands: Vec<Command>) -> Step {
    Step {
        name: name.into(),
        frames: vec![Frame {
            commands,
            ..Frame::default()
        }],
    }
}

/// A panel opened and drawn for a while.
fn panel(name: &str, p: Panel) -> Step {
    let mut frames: Vec<Frame> = (0..30).map(|_| Frame::default()).collect();
    frames[0].commands.push(Command::ShowPanel(p));
    Step {
        name: name.into(),
        frames,
    }
}

/// The steps for `d`'s model: its most used material and its most
/// repeated universe are the ones hidden, isolated and selected.
fn script(d: &Document) -> Vec<Step> {
    let m = &d.model;
    let x = &d.index;
    let material = x
        .materials()
        .into_iter()
        .filter(|&id| id != 0)
        .max_by_key(|&id| x.material_cells(id).len());
    let universe = (0..m.units().len())
        .filter(|&k| m.units()[k].universe != 0)
        .max_by_key(|&k| m.instances().count_unit(k as u32))
        .map(|k| m.units()[k].universe);
    let mut steps = vec![Step {
        name: "idle".into(),
        frames: (0..30).map(|_| Frame::default()).collect(),
    }];
    for (name, p) in [
        ("panel: statistics", Panel::Statistics),
        ("panel: volumes", Panel::Volumes),
        ("panel: problems", Panel::Problems),
        ("panel: fields", Panel::Fields),
        ("panel: inspector", Panel::Inspector),
        ("panel: FMESH", Panel::Mesh),
        ("panel: places", Panel::Places),
        ("panel: materials", Panel::Materials),
        ("panel: envelopes", Panel::Envelopes),
    ] {
        steps.push(panel(name, p));
    }
    let rule = match (universe, material) {
        (Some(u), Some(mat)) => format!("u {u} & m {mat}"),
        (_, Some(mat)) => format!("m {mat}"),
        _ => "c 1".into(),
    };
    let mut find = panel("find: a rule's matches", Panel::Find);
    find.frames[0].find_rule = Some(rule.clone());
    steps.push(find);
    let mut typing: Vec<Frame> = (1..=rule.len())
        .map(|n| Frame {
            palette: Some(rule[..n].to_string()),
            ..Frame::default()
        })
        .collect();
    typing.push(Frame {
        palette: Some(String::new()),
        ..Frame::default()
    });
    steps.push(Step {
        name: format!("palette: typing {rule}"),
        frames: typing,
    });
    if let Some(mat) = material {
        let q = Query::Materials(vec![mat]);
        steps.push(one(
            &format!("hide m {mat}"),
            vec![Edit::Hide(q.clone()).into()],
        ));
        steps.push(one("undo", vec![Command::Undo]));
        steps.push(one(
            &format!("isolate m {mat}"),
            vec![Edit::Isolate(Some(q)).into()],
        ));
        steps.push(one("isolate nothing", vec![Edit::Isolate(None).into()]));
    }
    if x.labels().is_some_and(|l| !l.is_empty()) {
        // Labels with an `e`: most of them, the slowest to gather.
        let q = Query::label("e");
        steps.push(one(
            "select $ e",
            vec![Command::Select(q.clone(), SelectMode::Replace)],
        ));
        steps.push(one("clear selection", vec![Command::ClearSelection]));
        steps.push(one("isolate $ e", vec![Edit::Isolate(Some(q)).into()]));
        steps.push(one("isolate nothing", vec![Edit::Isolate(None).into()]));
    }
    if let Some(u) = universe {
        let q = Query::Universes(vec![u]);
        steps.push(one(
            &format!("select u {u}"),
            vec![Command::Select(q, SelectMode::Replace)],
        ));
        steps.push(one("clear selection", vec![Command::ClearSelection]));
    }
    if d.levels > 1 {
        steps.push(one("level 0", vec![Edit::Level(Some(0)).into()]));
        steps.push(one("every level", vec![Edit::Level(None).into()]));
    }
    for (name, c) in [
        ("colour by material", Colouring::Material),
        ("colour realistic", Colouring::Realistic),
        ("colour by cell", Colouring::Cell),
    ] {
        steps.push(one(name, vec![Edit::Colouring(c).into()]));
    }
    if d.scene.field.is_some() {
        steps.push(Step {
            name: "field opacity drag".into(),
            frames: (0..=30)
                .map(|k| Frame {
                    commands: vec![Edit::FieldOpacity(1.0 - k as f32 / 40.0).into()],
                    ..Frame::default()
                })
                .collect(),
        });
    }
    steps
}
