use std::collections::HashMap;

use crate::backend::{Backend, ImageSource, Pixmap};
use crate::scenes::SceneId;

#[derive(Debug, Default)]
pub struct ResourceStore {
    scene_epochs: HashMap<SceneId, u64>,
    images: HashMap<(SceneId, u64, u64), ImageSource>,
}

impl ResourceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prepare_scene(&mut self, scene_id: SceneId, epoch: u64, backend: &mut dyn Backend) {
        let needs_reset = self.scene_epochs.get(&scene_id).copied() != Some(epoch);
        if needs_reset {
            self.clear_scene(scene_id, backend);
            self.scene_epochs.insert(scene_id, epoch);
        }
    }

    pub fn get_or_upload_image<F>(
        &mut self,
        scene_id: SceneId,
        epoch: u64,
        key: u64,
        backend: &mut dyn Backend,
        make_pixmap: F,
    ) -> ImageSource
    where
        F: FnOnce() -> Pixmap,
    {
        self.prepare_scene(scene_id, epoch, backend);
        let cache_key = (scene_id, epoch, key);
        self.images
            .entry(cache_key)
            .or_insert_with(|| backend.upload_image(make_pixmap()))
            .clone()
    }

    pub fn clear_scene(&mut self, scene_id: SceneId, backend: &mut dyn Backend) {
        let doomed: Vec<_> = self
            .images
            .keys()
            .copied()
            .filter(|(id, _, _)| *id == scene_id)
            .collect();
        for key in doomed {
            if let Some(image) = self.images.remove(&key) {
                backend.destroy_image(&image);
            }
        }
        self.scene_epochs.remove(&scene_id);
    }

    pub fn clear_all(&mut self, backend: &mut dyn Backend) {
        for (_, image) in self.images.drain() {
            backend.destroy_image(&image);
        }
        self.scene_epochs.clear();
    }
}
