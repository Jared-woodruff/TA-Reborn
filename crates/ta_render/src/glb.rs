//! A small reader for the GLB files (glTF 2.0 binary) TA Reborn's own Blender tools make:
//! the cockpits (`tools/cockpits/build.py`) and the intro's room (`tools/room/build.py`).
//!
//! It reads what those files hold, as optimised by gltfpack: nodes with their transforms
//! (translation, rotation and scale, or a matrix), triangle meshes whose attributes may be
//! quantised (KHR_mesh_quantization: normalised or plain integers, the node's transform undoing
//! the scale), materials, and extras. It is not a general glTF reader: no textures, skins,
//! animations, sparse accessors or external buffers.

use anyhow::{Context, Result, anyhow, bail};
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;

/// A GLB's JSON and its binary chunk.
pub struct Glb<'a> {
    pub doc: Value,
    bin: &'a [u8],
}

impl<'a> Glb<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Glb<'a>> {
        let u32_at = |o: usize| -> Result<u32> { bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| anyhow!("GLB: cut short")) };
        if bytes.get(0..4) != Some(b"glTF") || u32_at(4)? != 2 {
            bail!("GLB: not glTF 2.0 binary");
        }
        let (mut offset, mut json, mut bin) = (12usize, None, None);
        while offset + 8 <= bytes.len() {
            let len = u32_at(offset)? as usize;
            let kind = u32_at(offset + 4)?;
            let body = bytes.get(offset + 8..offset + 8 + len).ok_or_else(|| anyhow!("GLB: a chunk runs past the end"))?;
            match kind {
                0x4E4F_534A => json = Some(body),
                0x004E_4942 => bin = Some(body),
                _ => {}
            }
            offset += 8 + len;
        }
        let doc = serde_json::from_slice(json.ok_or_else(|| anyhow!("GLB: no JSON"))?).context("GLB: JSON")?;
        Ok(Glb { doc, bin: bin.unwrap_or(&[]) })
    }

    /// The default scene's root nodes.
    pub fn roots(&self) -> Vec<usize> {
        let scene = self.doc["scene"].as_u64().unwrap_or(0) as usize;
        self.doc["scenes"][scene]["nodes"].as_array().map(|n| n.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect()).unwrap_or_default()
    }

    pub fn node(&self, index: usize) -> &Value {
        &self.doc["nodes"][index]
    }

    /// The default scene's nodes, depth first in the file's order, each with its transform to
    /// the scene's space.
    pub fn walk(&self) -> Vec<(usize, Mat4)> {
        let mut out = Vec::new();
        let mut stack: Vec<(usize, Mat4)> = self.roots().into_iter().rev().map(|n| (n, Mat4::IDENTITY)).collect();
        while let Some((index, parent)) = stack.pop() {
            let node = self.node(index);
            let world = parent * node_matrix(node);
            out.push((index, world));
            if let Some(children) = node["children"].as_array() {
                stack.extend(children.iter().rev().filter_map(|c| c.as_u64()).map(|c| (c as usize, world)));
            }
        }
        out
    }

    /// How many components an accessor's elements have (1 for a scalar, 3 for a VEC3...).
    pub fn width(&self, accessor: usize) -> usize {
        match self.doc["accessors"][accessor]["type"].as_str() {
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT4") => 16,
            _ => 1,
        }
    }

    /// An accessor's components as floats (normalised integers scaled to 0..1, or -1..1),
    /// `width` per element.
    pub fn read_vec(&self, accessor: usize, width: usize) -> Result<Vec<f32>> {
        let a = &self.doc["accessors"][accessor];
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let kind = a["componentType"].as_u64().unwrap_or(5126);
        let normalized = a["normalized"].as_bool().unwrap_or(false);
        let size = match kind {
            5126 | 5125 => 4,
            5123 | 5122 => 2,
            _ => 1,
        };
        let (data, stride) = self.view_of(a, width * size)?;
        let mut out = Vec::with_capacity(count * width);
        for i in 0..count {
            for c in 0..width {
                let o = i * stride + c * size;
                let b = data.get(o..o + size).ok_or_else(|| anyhow!("GLB: an accessor runs past its buffer"))?;
                out.push(match kind {
                    5126 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    5123 => u16::from_le_bytes([b[0], b[1]]) as f32 / if normalized { 65535.0 } else { 1.0 },
                    5121 => b[0] as f32 / if normalized { 255.0 } else { 1.0 },
                    5122 => {
                        let v = i16::from_le_bytes([b[0], b[1]]) as f32;
                        if normalized { (v / 32767.0).max(-1.0) } else { v }
                    }
                    5120 => {
                        let v = b[0] as i8 as f32;
                        if normalized { (v / 127.0).max(-1.0) } else { v }
                    }
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                });
            }
        }
        Ok(out)
    }

    pub fn read_indices(&self, accessor: usize) -> Result<Vec<u32>> {
        let a = &self.doc["accessors"][accessor];
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let size = match a["componentType"].as_u64().unwrap_or(5125) {
            5125 => 4,
            5123 => 2,
            _ => 1,
        };
        let (data, stride) = self.view_of(a, size)?;
        (0..count)
            .map(|i| {
                let o = i * stride;
                let b = data.get(o..o + size).ok_or_else(|| anyhow!("GLB: indices run past their buffer"))?;
                Ok(match size {
                    4 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    2 => u16::from_le_bytes([b[0], b[1]]) as u32,
                    _ => b[0] as u32,
                })
            })
            .collect()
    }

    /// The bytes an accessor reads from, and the stride between its elements.
    fn view_of(&self, a: &Value, element: usize) -> Result<(&'a [u8], usize)> {
        let v = &self.doc["bufferViews"][a["bufferView"].as_u64().ok_or_else(|| anyhow!("GLB: an accessor without a buffer view"))? as usize];
        let start = v["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let end = v["byteOffset"].as_u64().unwrap_or(0) as usize + v["byteLength"].as_u64().unwrap_or(0) as usize;
        let stride = v["byteStride"].as_u64().map_or(element, |s| s as usize);
        Ok((self.bin.get(start..end).ok_or_else(|| anyhow!("GLB: a buffer view runs past the binary chunk"))?, stride))
    }
}

/// A node's own transform (to its parent's space).
pub fn node_matrix(node: &Value) -> Mat4 {
    let f = |v: &Value, n: usize, d: &[f32]| -> Vec<f32> { v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).filter(|a: &Vec<f32>| a.len() == n).unwrap_or_else(|| d.to_vec()) };
    if node["matrix"].is_array() {
        let m = f(&node["matrix"], 16, &Mat4::IDENTITY.to_cols_array());
        return Mat4::from_cols_slice(&m);
    }
    let t = f(&node["translation"], 3, &[0.0, 0.0, 0.0]);
    let r = f(&node["rotation"], 4, &[0.0, 0.0, 0.0, 1.0]);
    let s = f(&node["scale"], 3, &[1.0, 1.0, 1.0]);
    Mat4::from_scale_rotation_translation(Vec3::new(s[0], s[1], s[2]), Quat::from_xyzw(r[0], r[1], r[2], r[3]), Vec3::new(t[0], t[1], t[2]))
}

/// A material's metallic-roughness factors: base colour (linear rgb and alpha), metallic,
/// roughness, and its emission (linear, times its strength).
pub fn material_factors(m: &Value) -> ([f32; 4], f32, f32, Vec3) {
    let pbr = &m["pbrMetallicRoughness"];
    let num = |v: &Value, d: f32| v.as_f64().map_or(d, |x| x as f32);
    let base = pbr["baseColorFactor"].as_array().map_or([0.5, 0.5, 0.5, 1.0], |c| [num(&c[0], 0.5), num(&c[1], 0.5), num(&c[2], 0.5), c.get(3).map_or(1.0, |a| num(a, 1.0))]);
    let strength = num(&m["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"], 1.0);
    let emissive = m["emissiveFactor"].as_array().map_or(Vec3::ZERO, |c| Vec3::new(num(&c[0], 0.0), num(&c[1], 0.0), num(&c[2], 0.0))) * strength;
    (base, num(&pbr["metallicFactor"], 1.0), num(&pbr["roughnessFactor"], 1.0), emissive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_accessor_values_keep_their_range_unless_normalized() {
        for (kind, bytes, values, divisor) in [
            (5120, vec![128, 249, 0, 127], vec![-128.0, -7.0, 0.0, 127.0], 127.0),
            (5122, [i16::MIN, -300, 0, i16::MAX].into_iter().flat_map(i16::to_le_bytes).collect(), vec![-32768.0, -300.0, 0.0, 32767.0], 32767.0),
        ] {
            for normalized in [false, true] {
                let glb = Glb {
                    doc: serde_json::json!({
                        "bufferViews": [{"byteLength": bytes.len()}],
                        "accessors": [{"bufferView": 0, "componentType": kind, "count": 4, "type": "SCALAR", "normalized": normalized}]
                    }),
                    bin: &bytes,
                };
                let expected: Vec<f32> = values.iter().map(|&v: &f32| if normalized { (v / divisor).max(-1.0) } else { v }).collect();
                assert_eq!(glb.read_vec(0, 1).unwrap(), expected, "component {kind}, normalized {normalized}");
            }
        }
    }
}
