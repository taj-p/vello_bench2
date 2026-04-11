use crate::scenes::{ParamId, SceneId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnsupportedParamValue {
    scene_id: SceneId,
    param_id: ParamId,
    value: u32,
}

impl UnsupportedParamValue {
    pub(crate) const fn new(scene_id: SceneId, param_id: ParamId, value: u32) -> Self {
        Self {
            scene_id,
            param_id,
            value,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CapabilityProfile {
    scene_mask: u32,
    param_masks: [u64; SceneId::COUNT],
    unsupported_values: &'static [UnsupportedParamValue],
}

impl CapabilityProfile {
    pub(crate) const fn none() -> Self {
        Self {
            scene_mask: 0,
            param_masks: [0; SceneId::COUNT],
            unsupported_values: &[],
        }
    }

    pub(crate) const fn all() -> Self {
        Self {
            scene_mask: SceneId::ALL_MASK,
            param_masks: [ParamId::ALL_MASK; SceneId::COUNT],
            unsupported_values: &[],
        }
    }

    pub(crate) const fn allow_scenes(mut self, scenes: &[SceneId]) -> Self {
        let mut i = 0;
        while i < scenes.len() {
            self.scene_mask |= scenes[i].bit();
            i += 1;
        }
        self
    }

    pub(crate) const fn allow_params(mut self, scene_id: SceneId, params: &[ParamId]) -> Self {
        let row = scene_id.index();
        let mut i = 0;
        while i < params.len() {
            self.param_masks[row] |= params[i].bit();
            i += 1;
        }
        self
    }

    pub(crate) const fn deny_params(mut self, scene_id: SceneId, params: &[ParamId]) -> Self {
        let row = scene_id.index();
        let mut i = 0;
        while i < params.len() {
            self.param_masks[row] &= !params[i].bit();
            i += 1;
        }
        self
    }

    pub(crate) const fn with_unsupported_values(
        mut self,
        unsupported_values: &'static [UnsupportedParamValue],
    ) -> Self {
        self.unsupported_values = unsupported_values;
        self
    }

    pub(crate) fn supports_scene(self, scene_id: SceneId) -> bool {
        (self.scene_mask & scene_id.bit()) != 0
    }

    pub(crate) fn supports_param(self, scene_id: SceneId, param_id: ParamId) -> bool {
        (self.param_masks[scene_id.index()] & param_id.bit()) != 0
    }

    pub(crate) fn supports_param_value(
        self,
        scene_id: SceneId,
        param_id: ParamId,
        value: f64,
    ) -> bool {
        !self
            .unsupported_values
            .contains(&UnsupportedParamValue::new(
                scene_id,
                param_id,
                value as u32,
            ))
    }
}
