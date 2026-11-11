# naga 30.0.1, patched for TA Reborn

This folder is naga 30.0.1 as published on crates.io (built from gfx-rs/wgpu commit
`40f4a34ebaf56f9a046231f54125ad046239d3f3`), used in place of the crates.io copy through
`[patch.crates-io]` in the root `Cargo.toml`. naga is wgpu's shader translator; it is licensed
under MIT OR Apache-2.0 (`LICENSE.MIT`, `LICENSE.APACHE`), and the packages' third-party notices
list it as modified. Only `src/back/spv/mod.rs` and `src/back/spv/writer.rs` differ from the
published crate (the diff below); the crate's tooling files (`.cargo/`, `.gitattributes`,
`.gitignore`, `Cargo.lock`) are left out.

## The change

Vulkan requires the variable decorated `BuiltIn SampleMask` to be an array of 32-bit integers
(VUID-SampleMask-SampleMask-04359). naga 30 declares it with the type of the WGSL binding,
`@builtin(sample_mask) mask: u32`: a scalar. This is wgpu issue
[gfx-rs/wgpu#7311](https://github.com/gfx-rs/wgpu/issues/7311), open at the time of writing.

TA Reborn's billboards, units and 3D trees and rocks write a sample mask (`Covered` in
`crates/ta_render/src/shaders/common.wgsl`). On an AMD Radeon RX 9070 XT, the Vulkan driver
(`amdvlk64.dll`) failed with an access violation while building the sprites' pipelines, the
first pipelines in the renderer's build order that write one. AMD's shader compiler declares the
built-in as an array of 32-bit integers (`BUILTIN(SampleMask, 20, P, P, ai32)` in LLPC's
`lgc/interface/lgc/BuiltInDefs.h`). Other drivers accept the scalar.

The patch declares the variable as `array<u32, 1>` and has loads and stores go through
`OpAccessChain` to element 0 (`load_io_with_f16_polyfill`, `store_io_with_f16_polyfill`). The
SPIR-V means the same as before; only its declaration changes. Other backends (HLSL, MSL, GLSL)
are untouched.

`cargo test -p ta_render --lib sample_masks_are_arrays_in_spirv` checks every entry point that
writes a sample mask, compiled as a pipeline compiles it, declares the array and stores through
it. It fails without the patch.

## When wgpu is updated

- If wgpu's naga declares `SampleMask` as an array (gfx-rs/wgpu#7311 fixed), delete this folder,
  the `[patch.crates-io]` entry and the `exclude` in the root `Cargo.toml`.
- Otherwise copy the new naga version's crate here, keeping this file, and apply the diff below
  to it (`git apply --directory=third_party/naga <diff>`), then run the test above.

## The diff

```diff
--- a/src/back/spv/mod.rs
+++ b/src/back/spv/mod.rs
@@ -983,6 +983,11 @@
     /// when `StorageInputOutput16` capability is not available.
     io_f16_polyfills: f16_polyfill::F16IoPolyfill,
 
+    /// TA Reborn patch (gfx-rs/wgpu#7311): `SampleMask` I/O variables, declared as
+    /// `array<u32, 1>` as Vulkan requires, mapped to the pointer type of their element, which
+    /// loads and stores go through.
+    io_sample_mask_arrays: crate::FastHashMap<Word, Word>,
+
     /// Non semantic debug printf extension `OpExtInstImport`
     debug_printf: Option<Word>,
     pub(crate) ray_query_initialization_tracking: bool,
--- a/src/back/spv/writer.rs
+++ b/src/back/spv/writer.rs
@@ -118,6 +118,7 @@
             io_f16_polyfills: super::f16_polyfill::F16IoPolyfill::new(
                 options.use_storage_input_output_16,
             ),
+            io_sample_mask_arrays: crate::FastHashMap::default(),
             debug_printf: None,
             task_dispatch_limits: options.task_dispatch_limits,
             mesh_shader_primitive_indices_clamp: options.mesh_shader_primitive_indices_clamp,
@@ -212,6 +213,7 @@
             ray_tracing_functions: take(&mut self.ray_tracing_functions).reclaim(),
             has_ray_tracing_pipeline: false,
             io_f16_polyfills: take(&mut self.io_f16_polyfills).reclaim(),
+            io_sample_mask_arrays: take(&mut self.io_sample_mask_arrays).reclaim(),
             debug_printf: None,
         };
 
@@ -2993,6 +2995,20 @@
             self.io_f16_polyfills.register_io_var(id, f32_type_id);
 
             ptr_id
+        } else if matches!(binding, crate::Binding::BuiltIn(crate::BuiltIn::SampleMask)) {
+            // TA Reborn patch (gfx-rs/wgpu#7311): Vulkan requires the variable decorated
+            // `SampleMask` to be an array of 32-bit integers (VUID-SampleMask-SampleMask-04359),
+            // while the IR's is a `u32`. Declare `array<u32, 1>`; loads and stores go through its
+            // element (`load_io_with_f16_polyfill`, `store_io_with_f16_polyfill`).
+            let u32_type_id = self.get_u32_type_id();
+            let length_id = self.get_index_constant(1);
+            let array_type_id = self.id_gen.next();
+            Instruction::type_array(array_type_id, u32_type_id, length_id)
+                .to_words(&mut self.logical_layout.declarations);
+            let element_pointer_type_id = self.get_pointer_type_id(u32_type_id, class);
+            self.io_sample_mask_arrays
+                .insert(id, element_pointer_type_id);
+            self.get_pointer_type_id(array_type_id, class)
         } else {
             self.get_handle_pointer_type_id(ty, class)
         };
@@ -3368,6 +3384,19 @@
         target_type_id: Word,
     ) -> Word {
         let tmp = self.id_gen.next();
+        // (TA Reborn patch, gfx-rs/wgpu#7311: a `SampleMask` is read from its array's element.)
+        if let Some(&element_pointer_type_id) = self.io_sample_mask_arrays.get(&varying_id) {
+            let index_id = self.get_index_constant(0);
+            let element_id = self.id_gen.next();
+            body.push(Instruction::access_chain(
+                element_pointer_type_id,
+                element_id,
+                varying_id,
+                &[index_id],
+            ));
+            body.push(Instruction::load(target_type_id, tmp, element_id, None));
+            return tmp;
+        }
         if let Some(f32_ty) = self.io_f16_polyfills.get_f32_io_type(varying_id) {
             body.push(Instruction::load(f32_ty, tmp, varying_id, None));
             let converted = self.id_gen.next();
@@ -3391,6 +3420,19 @@
         varying_id: Word,
         value_id: Word,
     ) {
+        // (TA Reborn patch, gfx-rs/wgpu#7311: a `SampleMask` is written to its array's element.)
+        if let Some(&element_pointer_type_id) = self.io_sample_mask_arrays.get(&varying_id) {
+            let index_id = self.get_index_constant(0);
+            let element_id = self.id_gen.next();
+            body.push(Instruction::access_chain(
+                element_pointer_type_id,
+                element_id,
+                varying_id,
+                &[index_id],
+            ));
+            body.push(Instruction::store(element_id, value_id, None));
+            return;
+        }
         if let Some(f32_ty) = self.io_f16_polyfills.get_f32_io_type(varying_id) {
             let converted = self.id_gen.next();
             super::f16_polyfill::F16IoPolyfill::emit_f16_to_f32_conversion(
```
