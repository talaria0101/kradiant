//! The surface inspector

use kradiant::map::BrushContent;
use kradiant::render::TextureRegistry;

use super::{Condition, EditorState, Ui};

fn texture_size_for(texture: &str, tex_registry: &TextureRegistry) -> [f32; 2] {
    tex_registry
        .get(texture)
        .map(|rt| rt.size)
        .unwrap_or([256.0, 256.0])
}

fn fit_face_on_brush(
    brush: &mut kradiant::map::Brush,
    tex_registry: &TextureRegistry,
    face_idx: usize,
    sample_size: i32,
) {
    let polys = brush.polygons_for_drawing();
    if let (BrushContent::Convex(faces), Some(polys)) = (&mut brush.content, polys) {
        let Some((poly, _)) = polys.get(face_idx) else {
            return;
        };
        let Some(face) = faces.get_mut(face_idx) else {
            return;
        };
        let [tex_w, tex_h] = texture_size_for(&face.texture, tex_registry);
        face.fit_texture(poly, tex_w, tex_h, sample_size);
    }
}

fn fit_brush_on_texture(
    brush: &mut kradiant::map::Brush,
    tex_registry: &TextureRegistry,
    sample_size: i32,
) {
    let polys = brush.polygons_for_drawing();
    if let (BrushContent::Convex(faces), Some(polys)) = (&mut brush.content, polys) {
        for (face_idx, face) in faces.iter_mut().enumerate() {
            let Some((poly, _)) = polys.get(face_idx) else {
                continue;
            };
            let [tex_w, tex_h] = texture_size_for(&face.texture, tex_registry);
            face.fit_texture(poly, tex_w, tex_h, sample_size);
        }
    }
}

pub fn draw_surf_inspector(ui: &Ui, state: &mut EditorState) {
    ui.window("Surface Inspector")
        .size([480.0, 800.0], Condition::FirstUseEver)
        .build(|| {
            let core = &mut state.core;
            let tex_registry = &core.tex_registry;

            let Some(map) = core.map.as_mut() else {
                ui.text_disabled("No map loaded!");
                return;
            };

            let si = &mut state.sf_inputs;

            if core.edit_faces {
                if core.selected_faces.is_empty() {
                    ui.text_disabled("Select some faces!");
                    return;
                }

                ui.text(&format!("{} faces selected", core.selected_faces.len()));

                if let Some(sel) = core.selected_faces.last() {
                    if let Some(ent) = map.entities.get(sel.entity_idx) {
                        if let Some(brush) = ent.brushes.get(sel.brush_idx) {
                            if let Some(f) = brush.get_face(sel.face_idx) {
                                si.tex_in = f.texture;
                                si.hshift_in = f.params.shift.x;
                                si.vshift_in = f.params.shift.y;
                                si.hstretch_in = f.params.scale.x;
                                si.vstretch_in = f.params.scale.y;
                                si.rotate_in = f.params.rotate;
                            }
                        }
                    }
                }
            } else if !core.edit_edges && !core.edit_vertices {
                if core.selected_brushes.is_empty() {
                    ui.text_disabled("Select some brushes!");
                    return;
                }

                ui.text(&format!("{} brushes selected", core.selected_brushes.len()));

                if let Some(brush) = core.selected_brushes.last() {
                    if let Some(ent) = map.entities.get(brush.0) {
                        if let Some(brush) = ent.brushes.get(brush.1) {
                            if let Some(f) = brush.get_last_face() {
                                si.tex_in = f.texture;
                                si.hshift_in = f.params.shift.x;
                                si.vshift_in = f.params.shift.y;
                                si.hstretch_in = f.params.scale.x;
                                si.vstretch_in = f.params.scale.y;
                                si.rotate_in = f.params.rotate;
                            }
                        }
                    }
                }
            }
            ui.separator();

            let old_si = si.clone();

            let mut input_tex = si.tex_in.clone();

            ui.text("Texture");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            let tex_changed = ui
                .input_text("##texture", &mut input_tex)
                .hint("common/caulk")
                .capacity_hint(256)
                .enter_returns_true(true)
                .build();
            if tex_changed {
                si.tex_in = input_tex;
            }

            ui.text("Vertical Shift");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##vshift")
                .step(8)
                .build(&mut si.vshift_in);

            ui.text("Horizontal Shift");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##hshift")
                .step(8)
                .build(&mut si.hshift_in);

            ui.text("Vertical Stretch");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_float_config("##vstretch")
                .step(0.1)
                .build(&mut si.vstretch_in);

            ui.text("Horizontal Stretch");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_float_config("##hstretch")
                .step(0.1)
                .build(&mut si.hstretch_in);

            ui.text("Rotate");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##rotate")
                .step(45)
                .build(&mut si.rotate_in);
            si.rotate_in = si.rotate_in.clamp(0, 315);

            ui.text("Sample Size");
            ui.same_line();
            // ui.set_next_item_width(-1.0);
            ui.input_int_config("##sample_size")
                .step(4)
                .build(&mut si.sampsize_in);
            ui.same_line();
            if ui.button("Only") {
                si.sampsize_only = !si.sampsize_only;
            }

            ui.text("Value");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##value")
                .step(1)
                .step_fast(4)
                .build(&mut si.value_in);

            ui.separator();
            ui.set_next_item_width(-1.0);
            ui.text("Texturing");
            ui.separator();

            ui.text("Brush");
            ui.button_with_size("Axial", [64.0, 32.0]);
            ui.same_line();
            let brush_fit = ui.button_with_size("Fit", [64.0, 32.0]);

            ui.text("Patch");
            ui.button_with_size("CAP", [64.0, 32.0]);
            ui.same_line();
            ui.button_with_size("Set...", [64.0, 32.0]);
            ui.same_line();
            ui.button_with_size("Natural", [64.0, 32.0]);
            ui.same_line();
            let tok = ui.push_id("fit_patch");
            ui.button_with_size("Fit", [64.0, 32.0]);
            tok.end();

            if brush_fit {
                let sample_size = si.sampsize_in;
                if core.edit_faces {
                    for face_sel in core.selected_faces.clone() {
                        if let Some(ent) = map.entities.get_mut(face_sel.entity_idx) {
                            if let Some(brush) = ent.brushes.get_mut(face_sel.brush_idx) {
                                fit_face_on_brush(
                                    brush,
                                    tex_registry,
                                    face_sel.face_idx,
                                    sample_size,
                                );
                            }
                        }
                    }
                } else if !core.edit_edges && !core.edit_vertices {
                    for &(entity_idx, brush_idx) in &core.selected_brushes {
                        if let Some(ent) = map.entities.get_mut(entity_idx) {
                            if let Some(brush) = ent.brushes.get_mut(brush_idx) {
                                fit_brush_on_texture(brush, tex_registry, sample_size);
                            }
                        }
                    }
                }
                core.map_revision = core.map_revision.wrapping_add(1);
            }

            if &old_si != si {
                if core.edit_faces {
                    for face in &mut core.selected_faces {
                        if let Some(ent) = map.entities.get_mut(face.entity_idx) {
                            if let Some(brush) = ent.brushes.get_mut(face.brush_idx) {
                                if let BrushContent::Convex(faces) = &mut brush.content {
                                    faces[face.face_idx].apply_params(&si);
                                }
                            }
                        }
                    }
                } else if !core.edit_edges && !core.edit_vertices {
                    for sel in &mut core.selected_brushes {
                        if let Some(ent) = map.entities.get_mut(sel.0) {
                            if let Some(brush) = ent.brushes.get_mut(sel.1) {
                                brush.set_texture_params(si.clone());
                            }
                        }
                    }
                }
                core.map_revision = core.map_revision.wrapping_add(1);
            }
        });
}
