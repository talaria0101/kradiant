include!(concat!(env!("OUT_DIR"), "/generated_icons.rs"));
include!(concat!(env!("OUT_DIR"), "/generated_images.rs"));
include!(concat!(env!("OUT_DIR"), "/generated_themes.rs"));
//use editor_icons;

pub const EDITOR_VERSION: &str = env!("CARGO_PKG_VERSION");

use std::num::NonZeroU32;
use std::time::Instant;

use dear_imgui_glow::GlowRenderer;
use dear_imgui_rs::{Context, FontSource, TextureFormat};
use dear_imgui_winit::WinitPlatform;
use glam::Vec3;
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use kradiant::map::BrushContent;
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::config::EditorConfig;
use crate::icons::EditorIcons;
use crate::images::EditorImages;

#[macro_use]
mod ui;
mod config;
mod icons;
mod images;
mod theme;
mod util;

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App { state: None };
    event_loop.run_app(&mut app).expect("event loop error");
}

struct App {
    state: Option<AppState>,
}

struct AppState {
    window: Window,
    gl_surface: glutin::surface::Surface<WindowSurface>,
    gl_context: glutin::context::PossiblyCurrentContext,
    // GlowRenderer takes ownership of glow::Context, so we store it inside the renderer.
    // We still need a handle for clears — borrow it from the renderer when needed, or keep
    // a second Arc. Simplest: keep our own glow::Context for raw GL calls.
    gl: glow::Context,
    imgui: Context,
    platform: WinitPlatform,
    renderer: GlowRenderer,
    editor: ui::EditorState,
    last_frame: Instant,
    last_title: String,
    program: glow::Program,
    mvp_loc: glow::UniformLocation,
    color_loc: glow::UniformLocation,
    vbo: glow::Buffer,
    //ebo: glow::Buffer,
    vao: glow::NativeVertexArray,
    view2d_line_vertices: Vec<Vec3>,
    view2d_selected_vertices: Vec<Vec3>,
    view2d_grid_vertices: Vec<Vec3>,
    view2d_fbo: glow::Framebuffer,
    view2d_tex: glow::Texture,
    view2d_rbo: glow::Renderbuffer, // depth
    view2d_fbo_size: [u32; 2],
    view2d_cache: Option<View2dCache>,
    needs_redraw: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct View2dCache {
    axis: ui::Ortho,
    map_present: bool,
    map_ptr: usize,
    map_generation: u64,
    zoom: f32,
    // Expanded cull bounds (world units in the 2D plane coordinates).
    cull_left: f32,
    cull_right: f32,
    cull_top: f32,
    cull_bottom: f32,
}

impl AppState {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        // window + GL config ─
        let window_attrs = Window::default_attributes()
            .with_title("Kradiant Editor")
            .with_inner_size(LogicalSize::new(1280u32, 720u32));

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_depth_size(24);

        let (window, gl_config) = DisplayBuilder::new()
            .with_window_attributes(Some(window_attrs))
            .build(event_loop, template, |configs| {
                configs
                    .reduce(|acc, cfg| {
                        if cfg.num_samples() > acc.num_samples() {
                            cfg
                        } else {
                            acc
                        }
                    })
                    .expect("no suitable GL config")
            })
            .expect("DisplayBuilder failed");

        let window = window.expect("window creation failed");
        window.set_maximized(true);
        let icon_img = EditorIcons::get_image(&editor_icons::ICON_LOGO_DDS);
        let icon = winit::window::Icon::from_rgba(icon_img.rgba8, icon_img.width, icon_img.height)
            .unwrap();
        window.set_window_icon(Some(icon));

        // GL context ─
        let raw_handle = window.window_handle().unwrap().as_raw();
        let ctx_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(2, 1))))
            .build(Some(raw_handle));

        let not_current = unsafe {
            gl_config
                .display()
                .create_context(&gl_config, &ctx_attrs)
                .expect("failed to create GL context")
        };

        // GL surface
        let (width, height): (u32, u32) = window.inner_size().into();
        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_handle,
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        );

        let gl_surface = unsafe {
            gl_config
                .display()
                .create_window_surface(&gl_config, &surface_attrs)
                .expect("failed to create GL surface")
        };

        let gl_context = not_current
            .make_current(&gl_surface)
            .expect("make_current failed");

        // glow ─
        // We need two glow contexts: one for clears (kept here), one owned by the renderer.
        // glow::Context is Clone when backed by function pointers, so we build once and clone.
        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                let s = std::ffi::CString::new(s).unwrap();
                gl_config.display().get_proc_address(&s) as *const _
            })
        };

        // ImGui context
        let cfg_dir = util::get_config_dir().unwrap();
        let mut imgui = Context::create();
        imgui
            .set_ini_filename(Some(cfg_dir.join("editor_ui.ini")))
            .expect("Failed to load editor layout config");

        // Enable docking.
        let io = imgui.io_mut();
        io.set_config_flags(io.config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE);

        // platform backend
        //  attach_window(&window, HiDpiMode, &mut Context)
        let mut platform = WinitPlatform::new(&mut imgui);
        platform.attach_window(&window, dear_imgui_winit::HiDpiMode::Default, &mut imgui);

        // fonts
        imgui.fonts().add_font(&[
            /*FontSource::TtfData {
                data: include_bytes!("../assets/Nunito-Regular.ttf"),
                size_pixels: Some(18.0),
                config: None,
            },*/
            FontSource::TtfData {
                data: include_bytes!("../assets/fonts/Nunito-SemiBold.ttf"),
                size_pixels: Some(18.0),
                config: None,
            },
        ]);

        //  renderer ─
        // GlowRenderer::new takes ownership of glow::Context.
        // We clone gl before handing it over so we keep one for clears.
        let gl_for_renderer = unsafe {
            glow::Context::from_loader_function(|s| {
                let s = std::ffi::CString::new(s).unwrap();
                gl_config.display().get_proc_address(&s) as *const _
            })
        };
        let v = gl_for_renderer.version();
        let gl_info = format!("Using OpenGL {}.{} | {}", v.major, v.minor, v.vendor_info);

        // ── Simple shader for 2D wire (GL 2.1 compatible) ─────────────────────
        let program = unsafe {
            let vert = gl_for_renderer.create_shader(glow::VERTEX_SHADER).unwrap();
            gl_for_renderer.shader_source(
                vert,
                r#"
            #version 120
            attribute vec3 a_pos;
            uniform mat4 u_mvp;
            void main() { gl_Position = u_mvp * vec4(a_pos, 1.0); }
            "#,
            );
            gl_for_renderer.compile_shader(vert);

            if !gl_for_renderer.get_shader_compile_status(vert) {
                panic!("vert: {}", gl_for_renderer.get_shader_info_log(vert));
            }

            let frag = gl_for_renderer
                .create_shader(glow::FRAGMENT_SHADER)
                .unwrap();
            gl_for_renderer.shader_source(
                frag,
                r#"
            #version 120
            uniform vec4 u_color;
            void main() { gl_FragColor = u_color; }
            "#,
            );
            gl_for_renderer.compile_shader(frag);
            if !gl_for_renderer.get_shader_compile_status(frag) {
                panic!("frag: {}", gl_for_renderer.get_shader_info_log(frag));
            }

            let prog = gl_for_renderer.create_program().unwrap();
            gl_for_renderer.attach_shader(prog, vert);
            gl_for_renderer.attach_shader(prog, frag);
            gl_for_renderer.bind_attrib_location(prog, 0, "a_pos");
            gl_for_renderer.link_program(prog);
            prog
        };

        let mvp_loc = unsafe {
            gl_for_renderer
                .get_uniform_location(program, "u_mvp")
                .unwrap()
        };
        let color_loc = unsafe {
            gl_for_renderer
                .get_uniform_location(program, "u_color")
                .unwrap()
        };

        let vbo = unsafe { gl_for_renderer.create_buffer().unwrap() };
        //let ebo = unsafe { gl_for_renderer.create_buffer().unwrap() };
        let vao = unsafe { gl_for_renderer.create_vertex_array().unwrap() };
        unsafe {
            gl_for_renderer.bind_vertex_array(Some(vao));
        }

        let (view2d_fbo, view2d_tex, view2d_rbo) = unsafe {
            (
                gl_for_renderer.create_framebuffer().unwrap(),
                gl_for_renderer.create_texture().unwrap(),
                gl_for_renderer.create_renderbuffer().unwrap(),
            )
        };
        let view2d_fbo_size = [1u32, 1u32]; // resized on first frame

        unsafe {
            gl_for_renderer.bind_framebuffer(glow::FRAMEBUFFER, Some(view2d_fbo));

            gl_for_renderer.bind_texture(glow::TEXTURE_2D, Some(view2d_tex));
            gl_for_renderer.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                1,
                1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl_for_renderer.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl_for_renderer.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl_for_renderer.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(view2d_tex),
                0,
            );

            gl_for_renderer.bind_renderbuffer(glow::RENDERBUFFER, Some(view2d_rbo));
            gl_for_renderer.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT16, 1, 1);
            gl_for_renderer.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(view2d_rbo),
            );

            gl_for_renderer.bind_framebuffer(glow::FRAMEBUFFER, None);
        }

        let mut renderer =
            GlowRenderer::new(gl_for_renderer, &mut imgui).expect("GlowRenderer::new failed");

        let editor_splash = EditorImages::get_image(editor_images::IMG_SPLASH_DDS);
        let id_splash_img = renderer
            .register_texture(
                editor_splash.width,
                editor_splash.height,
                TextureFormat::RGBA32,
                &editor_splash.rgba8,
            )
            .expect("register editor image");
        let img_view_cycle = EditorIcons::get_image(editor_icons::ICON_VIEW_CHANGE_DDS);
        let id_icon_view_cycle = renderer
            .register_texture(
                img_view_cycle.width,
                img_view_cycle.height,
                TextureFormat::RGBA32,
                &img_view_cycle.rgba8,
            )
            .expect("register editor icon");

        let img_mouse_rotate = EditorIcons::get_image(editor_icons::ICON_SELECT_MOUSEROTATE_DDS);
        let id_mouse_rotate = renderer
            .register_texture(
                img_mouse_rotate.width,
                img_mouse_rotate.height,
                TextureFormat::RGBA32,
                &img_mouse_rotate.rgba8,
            )
            .expect("register editor icon");

        let img_free_scale = EditorIcons::get_image(editor_icons::ICON_SELECT_MOUSESCALE_DDS);
        let id_free_scale = renderer
            .register_texture(
                img_free_scale.width,
                img_free_scale.height,
                TextureFormat::RGBA32,
                &img_free_scale.rgba8,
            )
            .expect("register editor icon");

        let img_resize = EditorIcons::get_image(editor_icons::ICON_SELECT_MOUSERESIZE_DDS);
        let id_resize = renderer
            .register_texture(
                img_resize.width,
                img_resize.height,
                TextureFormat::RGBA32,
                &img_resize.rgba8,
            )
            .expect("register editor icon");

        let img_open = EditorIcons::get_image(editor_icons::ICON_FILE_OPEN_DDS);
        let id_open = renderer
            .register_texture(
                img_open.width,
                img_open.height,
                TextureFormat::RGBA32,
                &img_open.rgba8,
            )
            .expect("register editor icon");

        let img_save = EditorIcons::get_image(editor_icons::ICON_FILE_SAVE_DDS);
        let id_save = renderer
            .register_texture(
                img_save.width,
                img_save.height,
                TextureFormat::RGBA32,
                &img_save.rgba8,
            )
            .expect("register editor icon");

        let img_lock_x = EditorIcons::get_image(editor_icons::ICON_LOCK_X_DDS);
        let id_lock_x = renderer
            .register_texture(
                img_lock_x.width,
                img_lock_x.height,
                TextureFormat::RGBA32,
                &img_lock_x.rgba8,
            )
            .expect("register editor icon");

        let img_lock_y = EditorIcons::get_image(editor_icons::ICON_LOCK_Y_DDS);
        let id_lock_y = renderer
            .register_texture(
                img_lock_y.width,
                img_lock_y.height,
                TextureFormat::RGBA32,
                &img_lock_y.rgba8,
            )
            .expect("register editor icon");

        let img_lock_z = EditorIcons::get_image(editor_icons::ICON_LOCK_Z_DDS);
        let id_lock_z = renderer
            .register_texture(
                img_lock_z.width,
                img_lock_z.height,
                TextureFormat::RGBA32,
                &img_lock_z.rgba8,
            )
            .expect("register editor icon");

        let img_grid_snap = EditorIcons::get_image(editor_icons::ICON_SNAP_TO_GRID_DDS);
        let id_grid_snap = renderer
            .register_texture(
                img_grid_snap.width,
                img_grid_snap.height,
                TextureFormat::RGBA32,
                &img_grid_snap.rgba8,
            )
            .expect("register editor icon");

        let mut editor = ui::EditorState::default();
        editor.log_info(gl_info);

        // Apply configured theme on startup
        if !editor.themes.is_empty() {
            let idx = editor
                .config
                .active_theme
                .min(editor.themes.len().saturating_sub(1));
            if let Some(entry) = editor.themes.get(idx) {
                theme::apply_theme(&mut imgui, &entry.data);
                EditorConfig::update(&mut editor, "active_theme", idx);
            }
        }
        editor.palette = theme::palette_from_theme(
            &imgui,
            editor
                .themes
                .get(editor.config.active_theme)
                .map(|e| &e.data),
        );

        let view2d_imgui_tex =
            renderer
                .texture_map_mut()
                .register_texture(view2d_tex, 1, 1, TextureFormat::RGBA32);
        editor.view2d_tex_id = Some(view2d_imgui_tex);
        editor.images.splash = Some(id_splash_img);
        editor.icons.open = Some(id_open);
        editor.icons.save = Some(id_save);
        editor.icons.view_cycle = Some(id_icon_view_cycle);
        editor.icons.free_rotate = Some(id_mouse_rotate);
        editor.icons.free_scale = Some(id_free_scale);
        editor.icons.resize = Some(id_resize);
        editor.icons.lock_x = Some(id_lock_x);
        editor.icons.lock_y = Some(id_lock_y);
        editor.icons.lock_z = Some(id_lock_z);
        editor.icons.grid_snap = Some(id_grid_snap);

        Self {
            window,
            gl_surface,
            gl_context,
            gl,
            imgui,
            platform,
            renderer,
            editor,
            last_frame: Instant::now(),
            last_title: "Kradiant Editor".to_string(),
            program,
            mvp_loc,
            color_loc,
            vbo,
            //ebo,
            vao,
            view2d_line_vertices: vec![],
            view2d_selected_vertices: vec![],
            view2d_grid_vertices: vec![],
            view2d_fbo,
            view2d_tex,
            view2d_rbo,
            view2d_fbo_size,
            view2d_cache: None,
            needs_redraw: true,
        }
    }

    fn render(&mut self) {
        self.needs_redraw = false;

        let now = Instant::now();
        let delta = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.imgui.io_mut().set_delta_time(delta);

        let new_title = if !self.editor.map_path.is_empty() {
            format!("{} — Kradiant Editor", &self.editor.map_path)
        } else {
            "Kradiant Editor".to_string()
        };
        if new_title != self.last_title {
            self.window.set_title(&new_title);
            self.last_title = new_title;
        }

        if let Some(idx) = self.editor.pending_theme.take() {
            if let Some(entry) = self.editor.themes.get(idx) {
                theme::apply_theme(&mut self.imgui, &entry.data);
                EditorConfig::update(&mut self.editor, "active_theme", idx);
            }
        }

        self.editor.palette = theme::palette_from_theme(
            &self.imgui,
            self.editor
                .themes
                .get(self.editor.config.active_theme)
                .map(|e| &e.data),
        );

        self.platform.prepare_frame(&self.window, &mut self.imgui);
        let ui = self.imgui.frame();
        ui::draw_editor(ui, &mut self.editor, delta);

        self.platform.prepare_render(&mut self.imgui, &self.window);
        let draw_data = self.imgui.render();

        //self.renderer.render(draw_data).expect("imgui render failed");

        unsafe {
            use glow::HasContext;
            let (win_w, win_h): (u32, u32) = self.window.inner_size().into();

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.viewport(0, 0, win_w as i32, win_h as i32);
            let win_clear = self.editor.palette.window_clear;
            self.gl
                .clear_color(win_clear[0], win_clear[1], win_clear[2], win_clear[3]);
            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            //self.gl.enable(glow::DEPTH_TEST);
            self.gl.clear(glow::DEPTH_BUFFER_BIT);

            self.gl.use_program(Some(self.program));

            // 2d
            let r2 = self.editor.view2d_rect;
            if r2[2] > 10.0 && r2[3] > 10.0 {
                let fbo_w = r2[2] as u32;
                let fbo_h = r2[3] as u32;

                // Resize FBO texture if the panel size changed.
                if [fbo_w, fbo_h] != self.view2d_fbo_size {
                    self.view2d_fbo_size = [fbo_w, fbo_h];
                    self.gl
                        .bind_texture(glow::TEXTURE_2D, Some(self.view2d_tex));
                    self.gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA as i32,
                        fbo_w as i32,
                        fbo_h as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(None),
                    );
                    self.gl
                        .bind_renderbuffer(glow::RENDERBUFFER, Some(self.view2d_rbo));
                    self.gl.renderbuffer_storage(
                        glow::RENDERBUFFER,
                        glow::DEPTH_COMPONENT16,
                        fbo_w as i32,
                        fbo_h as i32,
                    );
                }

                // Draw into FBO.
                self.gl
                    .bind_framebuffer(glow::FRAMEBUFFER, Some(self.view2d_fbo));
                self.gl.viewport(0, 0, fbo_w as i32, fbo_h as i32);
                let view_bg = self.editor.palette.view2d_bg;
                self.gl
                    .clear_color(view_bg[0], view_bg[1], view_bg[2], view_bg[3]);
                self.gl
                    .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

                self.gl.use_program(Some(self.program));

                let zoom = self.editor.view2d_zoom.max(0.001);
                // `view2d_pan` is in pixels (screen space). Convert to world units here.
                // Also flip Y so positive world Y goes down (matching ImGui + screen_to_world).
                let half_w = fbo_w as f32 / (2.0 * zoom);
                let half_h = fbo_h as f32 / (2.0 * zoom);
                let pan_x = self.editor.view2d_pan[0] / zoom;
                let pan_y = self.editor.view2d_pan[1] / zoom;
                let view_left = -half_w - pan_x;
                let view_right = half_w - pan_x;
                let view_top = -half_h - pan_y;
                let view_bottom = half_h - pan_y;
                let ortho = glam::Mat4::orthographic_rh_gl(
                    view_left,
                    view_right,
                    view_bottom,
                    view_top,
                    -1024.0,
                    1024.0,
                );
                self.gl.uniform_matrix_4_f32_slice(
                    Some(&self.mvp_loc),
                    false,
                    &ortho.to_cols_array(),
                );

                let axis = self.editor.ortho_axis;

                // Draw grid under map lines.
                const MIN_MINOR_STEP_PX: f32 = 1.0;
                const MIN_MAJOR_STEP_PX: f32 = 8.0;
                const MAJOR_PROMOTE_FACTOR: f32 = 64.0;

                let base_minor_world = self.editor.config.grid_minor_step.max(1) as f32;
                let mut minor_world = base_minor_world;
                let mut major_world = 64.0f32.max(minor_world);

                // Promote major grid levels while it would draw "minor dense".
                for _ in 0..16 {
                    if major_world * zoom >= MIN_MAJOR_STEP_PX {
                        break;
                    }
                    minor_world = major_world;
                    major_world *= MAJOR_PROMOTE_FACTOR;
                }

                let major_step_px = major_world * zoom;
                let minor_step_px = minor_world * zoom;

                let draw_lines = |verts: &[Vec3], rgba: [f32; 4]| {
                    if verts.is_empty() {
                        return;
                    }
                    self.gl.uniform_4_f32_slice(Some(&self.color_loc), &rgba);
                    self.gl.bind_vertex_array(Some(self.vao));
                    self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                    self.gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        bytemuck::cast_slice(verts),
                        glow::STREAM_DRAW,
                    );
                    self.gl
                        .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
                    self.gl.enable_vertex_attrib_array(0);
                    self.gl.draw_arrays(glow::LINES, 0, verts.len() as i32);
                };

                if major_step_px >= MIN_MINOR_STEP_PX {
                    // Major grid
                    self.view2d_grid_vertices.clear();

                    let i0 = (view_left / major_world).floor() as i32 - 1;
                    let i1 = (view_right / major_world).ceil() as i32 + 1;
                    for i in i0..=i1 {
                        let x = i as f32 * major_world;
                        self.view2d_grid_vertices.push(Vec3::new(x, view_top, 0.0));
                        self.view2d_grid_vertices
                            .push(Vec3::new(x, view_bottom, 0.0));
                    }

                    let j0 = (view_top / major_world).floor() as i32 - 1;
                    let j1 = (view_bottom / major_world).ceil() as i32 + 1;
                    for j in j0..=j1 {
                        let y = j as f32 * major_world;
                        self.view2d_grid_vertices.push(Vec3::new(view_left, y, 0.0));
                        self.view2d_grid_vertices
                            .push(Vec3::new(view_right, y, 0.0));
                    }

                    draw_lines(
                        &self.view2d_grid_vertices,
                        self.editor.palette.view2d_grid_major,
                    );
                }

                if major_step_px >= MIN_MINOR_STEP_PX
                    && minor_step_px >= MIN_MINOR_STEP_PX
                    && (minor_world < major_world)
                {
                    // Minor grid (skip where major grid will draw, when aligned)
                    self.view2d_grid_vertices.clear();

                    let ratio = major_world / minor_world;
                    let major_factor = if (ratio - ratio.round()).abs() < 1.0e-4 && ratio >= 1.0 {
                        Some(ratio.round() as i32)
                    } else {
                        None
                    };

                    let i0 = (view_left / minor_world).floor() as i32 - 1;
                    let i1 = (view_right / minor_world).ceil() as i32 + 1;
                    for i in i0..=i1 {
                        if let Some(f) = major_factor {
                            if i.rem_euclid(f) == 0 {
                                continue;
                            }
                        }
                        let x = i as f32 * minor_world;
                        self.view2d_grid_vertices.push(Vec3::new(x, view_top, 0.0));
                        self.view2d_grid_vertices
                            .push(Vec3::new(x, view_bottom, 0.0));
                    }

                    let j0 = (view_top / minor_world).floor() as i32 - 1;
                    let j1 = (view_bottom / minor_world).ceil() as i32 + 1;
                    for j in j0..=j1 {
                        if let Some(f) = major_factor {
                            if j.rem_euclid(f) == 0 {
                                continue;
                            }
                        }
                        let y = j as f32 * minor_world;
                        self.view2d_grid_vertices.push(Vec3::new(view_left, y, 0.0));
                        self.view2d_grid_vertices
                            .push(Vec3::new(view_right, y, 0.0));
                    }

                    draw_lines(
                        &self.view2d_grid_vertices,
                        self.editor.palette.view2d_grid_minor,
                    );
                }

                // World origin axes under map lines.
                self.view2d_grid_vertices.clear();
                self.view2d_grid_vertices
                    .push(Vec3::new(0.0, view_top, 0.0));
                self.view2d_grid_vertices
                    .push(Vec3::new(0.0, view_bottom, 0.0));
                draw_lines(
                    &self.view2d_grid_vertices,
                    self.editor.palette.view2d_axis_y,
                );

                self.view2d_grid_vertices.clear();
                self.view2d_grid_vertices
                    .push(Vec3::new(view_left, 0.0, 0.0));
                self.view2d_grid_vertices
                    .push(Vec3::new(view_right, 0.0, 0.0));
                draw_lines(
                    &self.view2d_grid_vertices,
                    self.editor.palette.view2d_axis_x,
                );

                // brush geometry
                let map_ptr = self
                    .editor
                    .map
                    .as_ref()
                    .map(|m| (m as *const kradiant::map::Map) as usize)
                    .unwrap_or(0);
                let map_generation = self.editor.map.as_ref().map(|m| m.generation).unwrap_or(0);
                let map_present = self.editor.map.is_some();

                if !map_present {
                    self.view2d_line_vertices.clear();
                    self.view2d_cache = None;
                }

                let mut rebuild_view2d = false;
                match self.view2d_cache {
                    None => rebuild_view2d = map_present,
                    Some(cache) => {
                        if !map_present {
                            rebuild_view2d = true;
                        } else if cache.axis != axis
                            || cache.map_present != map_present
                            || cache.map_ptr != map_ptr
                            || cache.map_generation != map_generation
                            || (cache.zoom - zoom).abs() > 1.0e-6
                        {
                            rebuild_view2d = true;
                        } else {
                            // Reuse cached cull set while the current view stays within it.
                            if view_left < cache.cull_left
                                || view_right > cache.cull_right
                                || view_top < cache.cull_top
                                || view_bottom > cache.cull_bottom
                            {
                                rebuild_view2d = true;
                            }
                        }
                    }
                }

                if rebuild_view2d {
                    self.view2d_line_vertices.clear();

                    let margin_x = (view_right - view_left).abs() * 0.50;
                    let margin_y = (view_bottom - view_top).abs() * 0.50;
                    let cull_left = view_left - margin_x;
                    let cull_right = view_right + margin_x;
                    let cull_top = view_top - margin_y;
                    let cull_bottom = view_bottom + margin_y;

                    self.view2d_cache = Some(View2dCache {
                        axis,
                        map_present,
                        map_ptr,
                        map_generation,
                        zoom,
                        cull_left,
                        cull_right,
                        cull_top,
                        cull_bottom,
                    });
                }

                if rebuild_view2d {
                    if let Some(cache) = self.view2d_cache {
                        let view_dir = match axis {
                            ui::Ortho::XY => glam::Vec3::new(0.0, 0.0, -1.0),
                            ui::Ortho::XZ => glam::Vec3::new(0.0, -1.0, 0.0),
                            ui::Ortho::YZ => glam::Vec3::new(-1.0, 0.0, 0.0),
                        };
                        let view_min_x = cache.cull_left.min(cache.cull_right);
                        let view_max_x = cache.cull_left.max(cache.cull_right);
                        let view_min_y = cache.cull_top.min(cache.cull_bottom);
                        let view_max_y = cache.cull_top.max(cache.cull_bottom);

                        if let Some(map) = &mut self.editor.map {
                            for entity in &mut map.entities {
                                for brush in &mut entity.brushes {
                                    match &mut brush.content {
                                        BrushContent::Convex(_) => {
                                            let Some((aabb, polys)) = brush.get_polygons_and_aabb()
                                            else {
                                                continue;
                                            };

                                            // Coarse frustum cull by brush AABB before iterating faces/edges.
                                            let (a_min_x, a_max_x, a_min_y, a_max_y) = match axis {
                                                ui::Ortho::XY => (
                                                    aabb.min.x as f32,
                                                    aabb.max.x as f32,
                                                    aabb.min.y as f32,
                                                    aabb.max.y as f32,
                                                ),
                                                ui::Ortho::XZ => (
                                                    aabb.min.x as f32,
                                                    aabb.max.x as f32,
                                                    -(aabb.max.z as f32),
                                                    -(aabb.min.z as f32),
                                                ),
                                                ui::Ortho::YZ => (
                                                    aabb.min.y as f32,
                                                    aabb.max.y as f32,
                                                    -(aabb.max.z as f32),
                                                    -(aabb.min.z as f32),
                                                ),
                                            };
                                            if a_max_x < view_min_x
                                                || a_min_x > view_max_x
                                                || a_max_y < view_min_y
                                                || a_min_y > view_max_y
                                            {
                                                continue;
                                            }

                                            for (verts, _) in polys {
                                                if verts.len() < 3 {
                                                    continue;
                                                }

                                                // Backface cull faces relative to current ortho direction.
                                                let n = (verts[1] - verts[0])
                                                    .cross(verts[2] - verts[0]);
                                                let n_len = n.length();
                                                if n_len.is_finite() && n_len > 1e-6 {
                                                    let dot = (n / n_len).dot(view_dir);
                                                    if dot > 1e-4 {
                                                        continue;
                                                    }
                                                }

                                                for i in 0..verts.len() {
                                                    let a = verts[i];
                                                    let b = verts[(i + 1) % verts.len()];

                                                    // Frustum cull in projected 2D space.
                                                    let pa = util::project_to_2d(a, axis);
                                                    let pb = util::project_to_2d(b, axis);
                                                    let seg_min_x = pa[0].min(pb[0]);
                                                    let seg_max_x = pa[0].max(pb[0]);
                                                    let seg_min_y = pa[1].min(pb[1]);
                                                    let seg_max_y = pa[1].max(pb[1]);
                                                    if seg_max_x < view_min_x
                                                        || seg_min_x > view_max_x
                                                        || seg_max_y < view_min_y
                                                        || seg_min_y > view_max_y
                                                    {
                                                        continue;
                                                    }

                                                    self.view2d_line_vertices
                                                        .push(Vec3::new(pa[0], pa[1], 0.0));
                                                    self.view2d_line_vertices
                                                        .push(Vec3::new(pb[0], pb[1], 0.0));
                                                }
                                            }
                                        }
                                        BrushContent::Patch(patch) => {
                                            let Some((mesh, patch_aabb, edges)) =
                                                patch.get_mesh_aabb_wire()
                                            else {
                                                continue;
                                            };
                                            brush.aabb = patch_aabb.clone();
                                            let positions = mesh.positions.as_slice();

                                            let (a_min_x, a_max_x, a_min_y, a_max_y) = match axis {
                                                ui::Ortho::XY => (
                                                    patch_aabb.min.x as f32,
                                                    patch_aabb.max.x as f32,
                                                    patch_aabb.min.y as f32,
                                                    patch_aabb.max.y as f32,
                                                ),
                                                ui::Ortho::XZ => (
                                                    patch_aabb.min.x as f32,
                                                    patch_aabb.max.x as f32,
                                                    -(patch_aabb.max.z as f32),
                                                    -(patch_aabb.min.z as f32),
                                                ),
                                                ui::Ortho::YZ => (
                                                    patch_aabb.min.y as f32,
                                                    patch_aabb.max.y as f32,
                                                    -(patch_aabb.max.z as f32),
                                                    -(patch_aabb.min.z as f32),
                                                ),
                                            };
                                            if a_max_x < view_min_x
                                                || a_min_x > view_max_x
                                                || a_max_y < view_min_y
                                                || a_min_y > view_max_y
                                            {
                                                continue;
                                            }

                                            if positions.len() < 2 || edges.is_empty() {
                                                continue;
                                            }

                                            for &(a, b) in edges {
                                                let ia = a as usize;
                                                let ib = b as usize;
                                                if ia >= positions.len() || ib >= positions.len() {
                                                    continue;
                                                }
                                                let pa = util::project_to_2d(positions[ia], axis);
                                                let pb = util::project_to_2d(positions[ib], axis);

                                                let seg_min_x = pa[0].min(pb[0]);
                                                let seg_max_x = pa[0].max(pb[0]);
                                                let seg_min_y = pa[1].min(pb[1]);
                                                let seg_max_y = pa[1].max(pb[1]);
                                                if seg_max_x < view_min_x
                                                    || seg_min_x > view_max_x
                                                    || seg_max_y < view_min_y
                                                    || seg_min_y > view_max_y
                                                {
                                                    continue;
                                                }

                                                self.view2d_line_vertices
                                                    .push(Vec3::new(pa[0], pa[1], 0.0));
                                                self.view2d_line_vertices
                                                    .push(Vec3::new(pb[0], pb[1], 0.0));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !self.view2d_line_vertices.is_empty() {
                    draw_lines(
                        &self.view2d_line_vertices,
                        self.editor.palette.view2d_geometry,
                    );
                }

                self.view2d_selected_vertices.clear();
                if !self.editor.selected_brushes.is_empty() {
                    let view_min_x = view_left.min(view_right);
                    let view_max_x = view_left.max(view_right);
                    let view_min_y = view_top.min(view_bottom);
                    let view_max_y = view_top.max(view_bottom);
                    let view_dir = match axis {
                        ui::Ortho::XY => glam::Vec3::new(0.0, 0.0, -1.0),
                        ui::Ortho::XZ => glam::Vec3::new(0.0, -1.0, 0.0),
                        ui::Ortho::YZ => glam::Vec3::new(-1.0, 0.0, 0.0),
                    };

                    let preview_drag_mode = self.editor.view2d_drag_mode;
                    let preview_stretch_mode = self.editor.stretch_mode;
                    let preview_move_offset = self.editor.view2d_move_offset;
                    let preview_stretch = self.editor.view2d_stretch_preview_xform();
                    let preview_rotate = self.editor.view2d_rotate_preview_xform();
                    let preview_face = self.editor.view2d_face_stretch_preview();
                    let preview_point = |p: Vec3| -> Vec3 {
                        match preview_drag_mode {
                            ui::DragMode::MoveSelection => p + preview_move_offset,
                            ui::DragMode::StretchSelection => {
                                if preview_stretch_mode == ui::StretchMode::Scale {
                                    preview_stretch.map(|x| x.apply_point(p)).unwrap_or(p)
                                } else {
                                    p
                                }
                            }
                            ui::DragMode::NewBrush => p,
                            ui::DragMode::RotateSelection => {
                                preview_rotate.map(|r| r.apply_point(p)).unwrap_or(p)
                            }
                        }
                    };

                    for (entity_index, brush_index) in &self.editor.selected_brushes {
                        if let Some(map) = &mut self.editor.map {
                            if let Some(entity) = map.entities.get_mut(*entity_index) {
                                if let Some(brush) = entity.brushes.get_mut(*brush_index) {
                                    if matches!(&brush.content, BrushContent::Convex(_)) {
                                        if preview_drag_mode == ui::DragMode::StretchSelection
                                            && preview_stretch_mode == ui::StretchMode::Resize
                                        {
                                            if let Some((faces, delta)) = preview_face {
                                                if let Some(polys) = kradiant::editing::preview_convex_face_stretch_polys(
                                                    &*brush,
                                                    faces,
                                                    delta,
                                                    self.editor.config.grid_minor_step as i32,
                                                ) {
                                                    for (positions, _) in polys {
                                                        if positions.len() < 2 {
                                                            continue;
                                                        }
                                                        if positions.len() >= 3 {
                                                            let n = (positions[1] - positions[0])
                                                                .cross(positions[2] - positions[0]);
                                                            let n_len = n.length();
                                                            if n_len.is_finite() && n_len > 1e-6 {
                                                                let dot = (n / n_len).dot(view_dir);
                                                                if dot > 1e-4 {
                                                                    continue;
                                                                }
                                                            }
                                                        }
                                                        for i in 0..positions.len() {
                                                            let a = positions[i];
                                                            let b = positions[(i + 1) % positions.len()];
                                                            let pa = util::project_to_2d(a, axis);
                                                            let pb = util::project_to_2d(b, axis);

                                                            let seg_min_x = pa[0].min(pb[0]);
                                                            let seg_max_x = pa[0].max(pb[0]);
                                                            let seg_min_y = pa[1].min(pb[1]);
                                                            let seg_max_y = pa[1].max(pb[1]);
                                                            if seg_max_x < view_min_x
                                                                || seg_min_x > view_max_x
                                                                || seg_max_y < view_min_y
                                                                || seg_min_y > view_max_y
                                                            {
                                                                continue;
                                                            }

                                                            self.view2d_selected_vertices.push(Vec3::new(pa[0], pa[1], 0.0));
                                                            self.view2d_selected_vertices.push(Vec3::new(pb[0], pb[1], 0.0));
                                                        }
                                                    }
                                                    continue;
                                                }
                                            }
                                        }

                                        if let Some((_aabb, polys)) = brush.get_polygons_and_aabb()
                                        {
                                            for (positions, _) in polys {
                                                if positions.len() < 2 {
                                                    continue;
                                                }
                                                if positions.len() >= 3 {
                                                    let p0 = preview_point(positions[0]);
                                                    let p1 = preview_point(positions[1]);
                                                    let p2 = preview_point(positions[2]);
                                                    let n = (p1 - p0).cross(p2 - p0);
                                                    let n_len = n.length();
                                                    if n_len.is_finite() && n_len > 1e-6 {
                                                        let dot = (n / n_len).dot(view_dir);
                                                        if dot > 1e-4 {
                                                            continue;
                                                        }
                                                    }
                                                }
                                                for i in 0..positions.len() {
                                                    let a = positions[i];
                                                    let b = positions[(i + 1) % positions.len()];
                                                    let pa =
                                                        util::project_to_2d(preview_point(a), axis);
                                                    let pb =
                                                        util::project_to_2d(preview_point(b), axis);

                                                    let seg_min_x = pa[0].min(pb[0]);
                                                    let seg_max_x = pa[0].max(pb[0]);
                                                    let seg_min_y = pa[1].min(pb[1]);
                                                    let seg_max_y = pa[1].max(pb[1]);
                                                    if seg_max_x < view_min_x
                                                        || seg_min_x > view_max_x
                                                        || seg_max_y < view_min_y
                                                        || seg_min_y > view_max_y
                                                    {
                                                        continue;
                                                    }

                                                    self.view2d_selected_vertices
                                                        .push(Vec3::new(pa[0], pa[1], 0.0));
                                                    self.view2d_selected_vertices
                                                        .push(Vec3::new(pb[0], pb[1], 0.0));
                                                }
                                            }
                                        }
                                    } else if let BrushContent::Patch(patch) = &mut brush.content {
                                        let Some((mesh, patch_aabb, edges)) =
                                            patch.get_mesh_aabb_wire()
                                        else {
                                            continue;
                                        };
                                        brush.aabb = patch_aabb.clone();

                                        let (a_min_x, a_max_x, a_min_y, a_max_y) = match axis {
                                            ui::Ortho::XY => (
                                                patch_aabb.min.x as f32,
                                                patch_aabb.max.x as f32,
                                                patch_aabb.min.y as f32,
                                                patch_aabb.max.y as f32,
                                            ),
                                            ui::Ortho::XZ => (
                                                patch_aabb.min.x as f32,
                                                patch_aabb.max.x as f32,
                                                -(patch_aabb.max.z as f32),
                                                -(patch_aabb.min.z as f32),
                                            ),
                                            ui::Ortho::YZ => (
                                                patch_aabb.min.y as f32,
                                                patch_aabb.max.y as f32,
                                                -(patch_aabb.max.z as f32),
                                                -(patch_aabb.min.z as f32),
                                            ),
                                        };
                                        if a_max_x < view_min_x
                                            || a_min_x > view_max_x
                                            || a_max_y < view_min_y
                                            || a_min_y > view_max_y
                                        {
                                            continue;
                                        }

                                        let positions = mesh.positions.as_slice();
                                        for &(a, b) in edges {
                                            let ia = a as usize;
                                            let ib = b as usize;
                                            if ia >= positions.len() || ib >= positions.len() {
                                                continue;
                                            }
                                            let pa = util::project_to_2d(
                                                preview_point(positions[ia]),
                                                axis,
                                            );
                                            let pb = util::project_to_2d(
                                                preview_point(positions[ib]),
                                                axis,
                                            );

                                            let seg_min_x = pa[0].min(pb[0]);
                                            let seg_max_x = pa[0].max(pb[0]);
                                            let seg_min_y = pa[1].min(pb[1]);
                                            let seg_max_y = pa[1].max(pb[1]);
                                            if seg_max_x < view_min_x
                                                || seg_min_x > view_max_x
                                                || seg_max_y < view_min_y
                                                || seg_min_y > view_max_y
                                            {
                                                continue;
                                            }

                                            self.view2d_selected_vertices
                                                .push(Vec3::new(pa[0], pa[1], 0.0));
                                            self.view2d_selected_vertices
                                                .push(Vec3::new(pb[0], pb[1], 0.0));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !self.view2d_selected_vertices.is_empty() {
                    draw_lines(&self.view2d_selected_vertices, self.editor.selection_rgba);
                }

                self.gl.use_program(None);
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }

            self.gl.use_program(None);
        }

        self.renderer
            .render(draw_data)
            .expect("imgui render failed");
        self.gl_surface
            .swap_buffers(&self.gl_context)
            .expect("swap failed");
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            self.state = Some(AppState::new(event_loop));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };

        state
            .platform
            .handle_window_event(&mut state.imgui, &state.window, &event);

        match event {
            WindowEvent::CloseRequested => {
                let cfg = state.editor.config.clone();
                let _ = EditorConfig::save(cfg);
                event_loop.exit()
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                state.gl_surface.resize(
                    &state.gl_context,
                    NonZeroU32::new(size.width).unwrap(),
                    NonZeroU32::new(size.height).unwrap(),
                );
                state.needs_redraw = true;
            }
            WindowEvent::RedrawRequested => {
                state.render();
            }
            // Any input/UI event should schedule a redraw; we avoid continuous rendering when idle.
            WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Focused(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::Touch(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. } => {
                state.needs_redraw = true;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            if state.needs_redraw {
                state.window.request_redraw();
            }
        }
    }
}
