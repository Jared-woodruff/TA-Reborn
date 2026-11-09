// Ray tracing is off (see raytrace.rs): appended to common.wgsl.

// No ray-traced result: the shadow map and contact occlusion everywhere.
fn rt_visibility(frag: vec4<f32>) -> RtLight {
    return RtLight(false, 1.0, 1.0, 1.0);
}

// No ray tracing: the 3D features' shadows are in the shadow map with everything else's.
fn rt_feature_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    return 1.0;
}

// No ray-traced reflections: the shader's own sky and ground.
fn rt_reflection(frag: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}
