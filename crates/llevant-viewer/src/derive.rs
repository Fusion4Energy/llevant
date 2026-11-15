//! Scene edits brought to the 3D view off the UI thread.
//!
//! What a new scene needs before the 3D view can show it scales with the
//! model: the materials' looks (22 ms on the reference model), the skip of a
//! visibility ([`llevant::Model::skip`], 40 ms) and its upload, the palette
//! of every cell, the field's buffer. [`Deriver`] makes them on a thread of
//! its own while the view keeps showing the scene before; the document
//! applies each result as it lands ([`Derived`]), which only swaps buffers.
//!
//! Requests that queue up while one is made are merged: the thread derives
//! the latest scene against the last one it derived, so nothing a skipped
//! request changed is lost, and results are applied in the order made.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use llevant::{Model, Visibility};
use llevant_gpu::{Gpu, GpuModel};
use llevant_render::colours::ColourBy;
use llevant_render::field::Overlay;
use llevant_scene::index::ModelIndex;
use llevant_scene::materials::Looks;

use crate::scene::{Changes, Scene};
use crate::seen::Seen;
use crate::view3d::backend::Shown;
use crate::view3d::field::{field_words, DataKey};
use crate::view3d::palette_words;

/// A scene to bring the views to, with what the UI thread worked out of it
/// already (both cheap): its visibility and the field it draws.
pub struct Request {
    pub revision: u64,
    pub scene: Scene,
    pub vis: Visibility,
    pub field: Option<Overlay>,
    /// The field's data changed though the scene did not (it was read).
    pub field_read: bool,
}

/// What changed for the views, made ready to swap in.
pub struct Derived {
    /// Of the last request it covers.
    pub revision: u64,
    pub changes: Changes,
    /// The scene it was derived from.
    pub scene: Scene,
    pub looks: Option<Arc<Looks>>,
    /// The palette ([`palette_words`]) of these looks and colouring.
    pub palette: Option<(Arc<Looks>, ColourBy, Vec<u32>)>,
    pub shown: Option<Shown>,
    /// What the panels' eyes say of the new visibility.
    pub seen: Option<Arc<Seen>>,
    /// The field, with its buffer ([`field_words`]) when its data or
    /// colour ramp is not the one before (only the scale or opacity changed
    /// otherwise, which needs no buffer).
    pub field: Option<(Option<Overlay>, Option<Vec<u32>>)>,
    /// Why the scene could not be derived (nothing else is set then).
    pub error: Option<String>,
}

/// The thread that derives scenes, and the results not taken yet.
pub struct Deriver {
    tx: Sender<Request>,
    rx: Receiver<Derived>,
    sent: u64,
    done: u64,
}

impl Deriver {
    /// A thread deriving scenes of `model` from `scene` (shown already with
    /// `looks`), uploading visibilities for `gpu` too when given. `wake` is
    /// called when a result is ready (to repaint the window).
    pub fn new(
        model: Arc<Model>,
        index: Arc<ModelIndex>,
        gpu: Option<(Arc<Gpu>, Arc<GpuModel>)>,
        scene: Scene,
        looks: Arc<Looks>,
        wake: impl Fn() + Send + 'static,
    ) -> Deriver {
        let (tx, requests) = channel::<Request>();
        let (results, rx) = channel();
        std::thread::Builder::new()
            .name("llevant-derive".into())
            .spawn(move || {
                let (mut last, mut looks) = (scene, looks);
                // The field whose buffer was made last (held, so that its
                // data's address names it).
                let mut data: Option<Overlay> = None;
                while let Ok(mut req) = requests.recv() {
                    let mut field_read = req.field_read;
                    while let Ok(next) = requests.try_recv() {
                        field_read |= next.field_read;
                        req = next;
                    }
                    req.field_read = field_read;
                    let d = derive(
                        &model,
                        &index,
                        gpu.as_ref(),
                        &last,
                        &mut looks,
                        &mut data,
                        req,
                    );
                    if d.error.is_none() {
                        last = d.scene.clone();
                    }
                    if results.send(d).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .expect("a thread");
        Deriver {
            tx,
            rx,
            sent: 0,
            done: 0,
        }
    }

    /// Bring the views to `scene` (`vis` its visibility, `field` the field
    /// it draws); `field_read`: the field's data changed.
    pub fn send(
        &mut self,
        scene: Scene,
        vis: Visibility,
        field: Option<Overlay>,
        field_read: bool,
    ) {
        self.sent += 1;
        let _ = self.tx.send(Request {
            revision: self.sent,
            scene,
            vis,
            field,
            field_read,
        });
    }

    /// Results made since the last call, in order.
    pub fn poll(&mut self) -> Vec<Derived> {
        let out: Vec<Derived> = self.rx.try_iter().collect();
        if let Some(d) = out.last() {
            self.done = d.revision;
        }
        out
    }

    /// Whether a scene sent is still being derived.
    pub fn busy(&self) -> bool {
        self.done < self.sent
    }
}

/// What `req` changes since `last` (shown with `looks`, updated here).
fn derive(
    model: &Model,
    index: &ModelIndex,
    gpu: Option<&(Arc<Gpu>, Arc<GpuModel>)>,
    last: &Scene,
    looks: &mut Arc<Looks>,
    data: &mut Option<Overlay>,
    req: Request,
) -> Derived {
    let mut changes = Changes::between(last, &req.scene);
    changes.field_source |= req.field_read;
    let mut d = Derived {
        revision: req.revision,
        changes,
        scene: req.scene,
        looks: None,
        palette: None,
        shown: None,
        seen: None,
        field: None,
        error: None,
    };
    let made = || -> Result<(), String> {
        let mut new_looks = None;
        if changes.looks {
            new_looks = Some(Arc::new(
                d.scene
                    .looks(model)
                    .map_err(|e| format!("materials: {e}"))?,
            ));
        }
        let shown = if changes.visibility {
            let gpu = gpu.map(|(g, gm)| (&**g, &**gm));
            d.seen = Some(Arc::new(Seen::new(model, index, &d.scene, &req.vis)));
            Some(Shown::new(model, gpu, req.vis).map_err(|e| e.to_string())?)
        } else {
            None
        };
        if let Some(l) = &new_looks {
            *looks = l.clone();
        }
        if changes.looks || changes.colouring {
            let by: ColourBy = d.scene.colour_by.into();
            d.palette = Some((looks.clone(), by, palette_words(model, looks, by)));
        }
        if changes.field_source || changes.field_look {
            let made_for = data.as_ref().map(DataKey::of);
            let words = match &req.field {
                Some(f) if made_for != Some(DataKey::of(f)) => {
                    *data = Some(f.clone());
                    Some(field_words(f))
                }
                _ => None,
            };
            d.field = Some((req.field, words));
        }
        d.looks = new_looks;
        d.shown = shown;
        Ok(())
    };
    if let Err(e) = made() {
        d.error = Some(e);
        d.looks = None;
        d.palette = None;
        d.shown = None;
        d.seen = None;
        d.field = None;
    }
    d
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use llevant::Options;
    use llevant_scene::field::Field;

    use super::*;
    use crate::state::{Colouring, FieldSource, FieldState};

    fn model() -> Arc<Model> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/mcnp-oracle/models/universes.i"
        );
        Arc::new(Model::load(path, &Options::default(), None).unwrap())
    }

    fn deriver(m: &Arc<Model>) -> Deriver {
        let looks = Arc::new(Looks::new(m, &[]).unwrap());
        let x = Arc::new(ModelIndex::new(m));
        Deriver::new(m.clone(), x, None, Scene::default(), looks, || {})
    }

    /// Every result until none is being made.
    fn results(d: &mut Deriver) -> Vec<Derived> {
        let start = Instant::now();
        let mut out = Vec::new();
        while d.busy() {
            assert!(start.elapsed() < Duration::from_secs(20), "deriving");
            out.extend(d.poll());
            std::thread::sleep(Duration::from_millis(1));
        }
        out
    }

    #[test]
    fn merged_requests_lose_no_change() {
        let m = model();
        let mut d = deriver(&m);
        let vis = Visibility::solid(&m);
        let mut s = Scene {
            colour_by: Colouring::Material,
            ..Scene::default()
        };
        d.send(s.clone(), vis.clone(), None, false);
        s.effects.ambient_occlusion = !s.effects.ambient_occlusion;
        d.send(s.clone(), vis, None, false);
        let out = results(&mut d);
        assert!(out.iter().all(|r| r.error.is_none()));
        assert!(out
            .iter()
            .any(|r| r.changes.colouring && r.palette.is_some()));
        assert!(out.iter().any(|r| r.changes.effects));
        assert_eq!(out.last().unwrap().scene, s);
    }

    #[test]
    fn only_new_field_data_makes_a_buffer() {
        let m = model();
        let mut d = deriver(&m);
        let vis = Visibility::solid(&m);
        let values = m.cells().iter().map(|c| (c.id, c.id as f64));
        let f = Overlay::new(Arc::new(Field::cells(&m, "c", values).unwrap()));
        let mut s = Scene {
            field: Some(FieldState::new(FieldSource::new("c.csv".into()))),
            ..Scene::default()
        };
        d.send(s.clone(), vis.clone(), Some(f.clone()), false);
        let first = results(&mut d).pop().unwrap();
        assert!(first.field.unwrap().1.is_some());
        // Only the opacity: no buffer.
        s.field.as_mut().unwrap().opacity = 0.5;
        let mut g = f.clone();
        g.look.opacity = 0.5;
        d.send(s.clone(), vis.clone(), Some(g.clone()), false);
        let second = results(&mut d).pop().unwrap();
        let (shown, words) = second.field.unwrap();
        assert_eq!(shown.unwrap().look.opacity, 0.5);
        assert!(words.is_none());
        // The same source read again: new data, a buffer.
        let again = Overlay {
            field: Arc::new((*f.field).clone()),
            look: g.look,
        };
        d.send(s, vis, Some(again), true);
        assert!(results(&mut d).pop().unwrap().field.unwrap().1.is_some());
    }
}
