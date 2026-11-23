//! Directional lighting uniform shared by the schematic scene pipelines.

use crate::render::math::Vec3;

/// Uniform byte length of the shader `Light` struct: two 16-byte `vec4`s.
pub const LIGHT_UNIFORM_BYTES: u64 = 32;

/// Per-frame lighting values for the schematic scene pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLighting {
    /// Direction from a surface toward the light; the shader normalizes it.
    pub direction: Vec3,
    pub ambient: f32,
    pub sunlight: f32,
}

impl Default for FrameLighting {
    fn default() -> Self {
        Self {
            direction: [1.0, 1.4, 0.8],
            ambient: 0.68,
            sunlight: 0.32,
        }
    }
}

impl FrameLighting {
    /// Packed uniform bytes: `direction.xyz` and `ambient` form the first `vec4`, `sunlight`
    /// and the unused tail form the second.
    pub fn encode(self) -> [u8; LIGHT_UNIFORM_BYTES as usize] {
        let mut bytes = [0u8; LIGHT_UNIFORM_BYTES as usize];
        for (index, value) in self
            .direction
            .into_iter()
            .chain([self.ambient, self.sunlight])
            .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::FrameLighting;

    #[test]
    fn encode_places_levels_in_the_shader_vec4_slots() {
        let bytes = FrameLighting::default().encode();
        let floats: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|value| f32::from_le_bytes(*value))
            .collect();
        assert_eq!(&floats[..5], &[1.0, 1.4, 0.8, 0.68, 0.32]);
        assert!(floats[5..].iter().all(|value| *value == 0.0));
    }
}
