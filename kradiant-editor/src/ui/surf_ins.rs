//! The surface inspector

use kradiant::map::BrushContent;

use super::{EditorState, Ui, Condition};

pub fn draw_surf_inspector(ui: &Ui, state: &mut EditorState)
{
    ui.window("Surface Inspector")
        .size([480.0, 800.0], Condition::FirstUseEver)
        .build(|| {
            let core = &mut state.core;

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

                if let Some(face) = core.selected_faces.last() {
                    if let Some(ent) = map.entities.get(face.entity_idx) {
                        if let Some(brush) = ent.brushes.get(face.brush_idx) {
                            si.tex_in = brush.get_texture_last();
                        }
                    }
                }
            }
            else if !core.edit_edges && !core.edit_vertices {
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

            ui.text("Texture");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_text("##texture", &mut si.tex_in).hint("common/caulk").capacity_hint(256).enter_returns_true(true).build();

            ui.text("Vertical Shift");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##vshift").step(8).build(&mut si.vshift_in);

            ui.text("Horizontal Shift");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##hshift").step(8).build(&mut si.hshift_in);

            ui.text("Vertical Stretch");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_float_config("##vstretch").step(0.1).build(&mut si.vstretch_in);

            ui.text("Horizontal Stretch");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_float_config("##hstretch").step(0.1).build(&mut si.hstretch_in);

            ui.text("Rotate");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##rotate").step(45).build(&mut si.rotate_in);
            si.rotate_in = si.rotate_in.clamp(0, 315);

            ui.text("Sample Size");
            ui.same_line();
            // ui.set_next_item_width(-1.0);
            ui.input_int_config("##sample_size").step(4).build(&mut si.sampsize_in);
            ui.same_line();
            if ui.button("Only") {
                si.sampsize_only = !si.sampsize_only;
            }

            ui.text("Value");
            ui.same_line();
            ui.set_next_item_width(-1.0);
            ui.input_int_config("##value").step(1).step_fast(4).build(&mut si.value_in);

            if &old_si != si {
                state.console.warn("something changed");
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
                }
                else if !core.edit_edges && !core.edit_vertices {
                    for sel in &mut core.selected_brushes {
                        if let Some(ent) = map.entities.get_mut(sel.0) {
                            if let Some(brush) = ent.brushes.get_mut(sel.1) {
                                brush.set_texture_params(si.clone());
                            }
                        }
                    }
                }
            }

            core.map_revision = core.map_revision.wrapping_add(1);
        });

}
