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
use kradiant::assets::AssetDbOptions;
use kradiant::loader::asset_loader;
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::dpi::PhysicalPosition;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::config::{save as save_config, update as update_config};
use crate::icons::EditorIcons;
use crate::images::EditorImages;
use kradiant::editor::config::RenderMode;
use kradiant::render::{RenderBackend, Viewport2D, Viewport3D};

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
    /*view2d_line_vertices: Vec<Vec3>,
    view2d_selected_vertices: Vec<Vec3>,
    view2d_grid_vertices: Vec<Vec3>,
    view2d_fbo: glow::Framebuffer,
    view2d_tex: glow::Texture,
    view2d_rbo: glow::Renderbuffer, // depth
    view2d_fbo_size: [u32; 2],
    view2d_cache: Option<View2dCache>,*/
    vp2d: Viewport2D,
    vp3d: Viewport3D,
    needs_redraw: bool,
    last_render_mode: RenderMode,
    cursor_grabbed: bool,
    ignore_next_cursor_move: bool,
}

/// Register a texture image with the renderer.
fn register_texture(
    renderer: &mut GlowRenderer,
    img: &kradiant::texture::TextureImage,
    label: &str,
) -> dear_imgui_rs::TextureId {
    renderer
        .register_texture(img.width, img.height, TextureFormat::RGBA32, &img.rgba8)
        .unwrap_or_else(|_| panic!("failed to register {}", label))
}

/// Upload texture to GPU with mipmaps and filtering
///
/// Filtering modes copied from q3radiant:
/// https://github.com/id-Software/Quake-III-Arena/blob/master/q3radiant/TexWnd.cpp#L302-L324
///
unsafe fn upload_texture_mipmaps(
    gl: &glow::Context,
    size: [u32; 2],
    rgba: &[u8],
    mode: &RenderMode,
) -> glow::Texture {
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));

        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            size[0] as i32,
            size[1] as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(rgba)),
        );

        match mode {
            RenderMode::Nearest => {
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::NEAREST as i32,
                );
            }
            RenderMode::NearestMipmap => {
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST_MIPMAP_NEAREST as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::NEAREST as i32,
                );
            }
            RenderMode::Linear => {
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST_MIPMAP_LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
            }
            RenderMode::Bilinear => {
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
            }
            RenderMode::BilinearMipmap => {
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR_MIPMAP_NEAREST as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
            }
            RenderMode::Trilinear => {
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR_MIPMAP_LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
            }
            _ => {
                return tex;
            }
        }

        // repeat
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);

        tex
    }
}

/// Compile the 2D wire shader program (GL 2.1 compatible).
/// Returns (program, mvp_uniform_location, color_uniform_location).
fn compile_wire_shader(
    gl: &glow::Context,
) -> (glow::Program, glow::UniformLocation, glow::UniformLocation) {
    unsafe {
        let vert = gl.create_shader(glow::VERTEX_SHADER).unwrap();
        gl.shader_source(
            vert,
            include_str!("glsl/wire_vert.glsl"),
        );
        gl.compile_shader(vert);

        if !gl.get_shader_compile_status(vert) {
            panic!(
                "vertex shader compilation failed: {}",
                gl.get_shader_info_log(vert)
            );
        }

        let frag = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
        gl.shader_source(
            frag,
            include_str!("glsl/wire_frag.glsl"),
        );
        gl.compile_shader(frag);

        if !gl.get_shader_compile_status(frag) {
            panic!(
                "fragment shader compilation failed: {}",
                gl.get_shader_info_log(frag)
            );
        }

        let program = gl.create_program().unwrap();
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.bind_attrib_location(program, 0, "a_pos");
        gl.link_program(program);

        let mvp_loc = gl.get_uniform_location(program, "u_mvp").unwrap();
        let color_loc = gl.get_uniform_location(program, "u_color").unwrap();

        (program, mvp_loc, color_loc)
    }
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
            .with_window_attributes(Some(window_attrs.clone()))
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
        let req_gl_v = Version::new(2, 1);
        let ctx_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(req_gl_v)))
            .build(Some(raw_handle));

        let not_current =
            match unsafe { gl_config.display().create_context(&gl_config, &ctx_attrs) } {
                Ok(ctx) => ctx,
                Err(e) => {
                    let versions = [
                        //(4, 6), (4, 5), (4, 4), (4, 3), (4, 2), (4, 1), (4, 0),
                        //(3, 3), (3, 2), (3, 1), (3, 0), (2, 1)
                        (2, 0),
                        (1, 5),
                        (1, 4),
                        (1, 3),
                        (1, 2),
                    ];

                    let highest_supported = versions.iter().find_map(|&(major, minor)| {
                        let attrs = ContextAttributesBuilder::new()
                            .with_context_api(ContextApi::OpenGl(Some(Version::new(major, minor))))
                            .build(Some(raw_handle));
                        unsafe { gl_config.display().create_context(&gl_config, &attrs).ok() }
                            .map(|_| format!("{major}.{minor}"))
                    });

                    let version_info = match highest_supported {
                        Some(v) => format!("Your seems to support up to OpenGL {v}"),
                        None => "Could not determine supported OpenGL version".to_string(),
                    };

                    let msg =
                        format!("Kradiant requires OpenGL 2.1.\n{version_info}\n\nGLX error: {e}");

                    rfd::MessageDialog::new()
                        .set_title("Kradiant — OpenGL Error")
                        .set_description(&msg)
                        .set_level(rfd::MessageLevel::Error)
                        .show();

                    std::process::exit(1);
                }
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
            FontSource::TtfData {
                data: include_bytes!("../assets/fonts/Nunito-SemiBold.ttf"),
                size_pixels: Some(18.0),
                config: None,
            },
            FontSource::TtfData {
                data: include_bytes!("../assets/fonts/NotoEmoji-SemiBold.ttf"),
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

        // Compile the 2D wire shader program
        let (program, mvp_loc, color_loc) = compile_wire_shader(&gl_for_renderer);

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

        let (view3d_fbo, view3d_tex, view3d_rbo) = unsafe {
            (
                gl_for_renderer.create_framebuffer().unwrap(),
                gl_for_renderer.create_texture().unwrap(),
                gl_for_renderer.create_renderbuffer().unwrap(),
            )
        };
        let view3d_fbo_size = [1u32, 1u32]; // resized on first frame

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

            gl_for_renderer.bind_framebuffer(glow::FRAMEBUFFER, Some(view3d_fbo));

            gl_for_renderer.bind_texture(glow::TEXTURE_2D, Some(view3d_tex));
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
                Some(view3d_tex),
                0,
            );

            gl_for_renderer.bind_renderbuffer(glow::RENDERBUFFER, Some(view3d_rbo));
            gl_for_renderer.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT16, 1, 1);
            gl_for_renderer.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(view3d_rbo),
            );

            gl_for_renderer.bind_framebuffer(glow::FRAMEBUFFER, None);
        }

        let mut renderer =
            GlowRenderer::new(gl_for_renderer, &mut imgui).expect("GlowRenderer::new failed");

        let editor_splash = EditorImages::get_image(editor_images::IMG_SPLASH_DDS);
        let id_splash_img = register_texture(&mut renderer, &editor_splash, "editor splash image");

        // Register all icons
        let icons_data = [
            (editor_icons::ICON_VIEW_CHANGE_DDS, "view_cycle"),
            (editor_icons::ICON_SELECT_MOUSEROTATE_DDS, "mouse_rotate"),
            (editor_icons::ICON_SELECT_MOUSESCALE_DDS, "free_scale"),
            (editor_icons::ICON_SELECT_MOUSERESIZE_DDS, "resize"),
            (editor_icons::ICON_FILE_OPEN_DDS, "open"),
            (editor_icons::ICON_FILE_SAVE_DDS, "save"),
            (editor_icons::ICON_LOCK_X_DDS, "lock_x"),
            (editor_icons::ICON_LOCK_Y_DDS, "lock_y"),
            (editor_icons::ICON_LOCK_Z_DDS, "lock_z"),
            (editor_icons::ICON_SNAP_TO_GRID_DDS, "grid_snap"),
            (editor_icons::ICON_MODIFY_FACES_DDS, "edit_face"),
            (editor_icons::ICON_MODIFY_EDGES_DDS, "edit_edge"),
            (editor_icons::ICON_MODIFY_VERTICES_DDS, "edit_vertex"),
            (editor_icons::ICON_DONATE_DDS, "donate"),
        ];

        let mut icon_ids = Vec::new();
        for (icon_data, _name) in &icons_data {
            let img = EditorIcons::get_image(icon_data);
            icon_ids.push(register_texture(&mut renderer, &img, "editor icon"));
        }

        let mut editor = ui::EditorState::default();
        editor.console.info(gl_info);

        // Initialize texture browser with game main directory
        if !editor.core.config.paths.main.as_os_str().is_empty() {
            let log = editor.tex_browser.init(&editor.core.config.paths.main);
            for entry in log {
                if entry.starts_with("Fail") {
                    editor.console.error(entry);
                }
                else {
                    editor.console.info(entry);
                }
            }
        }

        // Apply configured theme on startup
        if !editor.themes.is_empty() {
            let idx = editor
                .core
                .config
                .misc
                .theme
                .min(editor.themes.len().saturating_sub(1));
            if let Some(entry) = editor.themes.get(idx) {
                theme::apply_theme(&mut imgui, &entry.data);
                update_config(
                    &mut editor.core.config,
                    "active_theme",
                    idx,
                    &mut editor.console,
                    &mut editor.core.view_config_rev,
                );
            }
        }
        editor.core.palette = theme::palette_from_theme(
            &imgui,
            editor
                .themes
                .get(editor.core.config.misc.theme)
                .map(|e| &e.data),
        );

        let view2d_imgui_tex =
            renderer
                .texture_map_mut()
                .register_texture(view2d_tex, 1, 1, TextureFormat::RGBA32);

        let view3d_imgui_tex =
            renderer
                .texture_map_mut()
                .register_texture(view3d_tex, 1, 1, TextureFormat::RGBA32);

        editor.view2d.tex_id = Some(view2d_imgui_tex);
        editor.view3d.tex_id = Some(view3d_imgui_tex);
        editor.images.splash = Some(id_splash_img);
        editor.icons.open = icon_ids.get(4).copied();
        editor.icons.save = icon_ids.get(5).copied();
        editor.icons.view_cycle = icon_ids.get(0).copied();
        editor.icons.free_rotate = icon_ids.get(1).copied();
        editor.icons.free_scale = icon_ids.get(2).copied();
        editor.icons.resize = icon_ids.get(3).copied();
        editor.icons.lock_x = icon_ids.get(6).copied();
        editor.icons.lock_y = icon_ids.get(7).copied();
        editor.icons.lock_z = icon_ids.get(8).copied();
        editor.icons.grid_snap = icon_ids.get(9).copied();
        editor.icons.edit_face = icon_ids.get(10).copied();
        editor.icons.edit_edge = icon_ids.get(11).copied();
        editor.icons.edit_vertex = icon_ids.get(12).copied();
        editor.icons.donate = icon_ids.get(13).copied();

        let vp2d = Viewport2D::new(view2d_fbo, view2d_fbo_size, view2d_rbo, view2d_tex);

        let vp3d = Viewport3D::new(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            view3d_fbo,
            view3d_fbo_size,
            view3d_tex,
            view3d_rbo,
            None,
        );

        let last_render_mode = editor.core.config.view.rendermode.clone();

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
            vp2d,
            vp3d,
            needs_redraw: true,
            last_render_mode,
            cursor_grabbed: false,
            ignore_next_cursor_move: false,
        }
    }

    fn render(&mut self) {
        self.needs_redraw = false;

        let now = Instant::now();
        let delta = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.imgui.io_mut().set_delta_time(delta);

        let new_title = if !self.editor.core.map_path.is_empty() {
            format!("{} — Kradiant Editor", &self.editor.core.map_path)
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
                update_config(
                    &mut self.editor.core.config,
                    "active_theme",
                    idx,
                    &mut self.editor.console,
                    &mut self.editor.core.view_config_rev,
                );
            }
        }

        self.editor.core.palette = theme::palette_from_theme(
            &self.imgui,
            self.editor
                .themes
                .get(self.editor.core.config.misc.theme)
                .map(|e| &e.data),
        );

        let current_render_mode = self.editor.core.config.view.rendermode.clone();
        if self.last_render_mode != current_render_mode {
            self.editor.tex_browser.clear_render_texture_caches();
            self.editor.core.tex_registry.clear();
            if let Some(map) = self.editor.core.map.as_ref() {
                let shader_db = self.editor.core.shader_db.as_ref();
                for mat in map.collect_used_materials(shader_db) {
                    self.editor.core.tex_registry.request(mat);
                }
            }
            self.last_render_mode = current_render_mode;
        }

        // Bridge shared renderer texture requests into the editor-side loader/uploader.
        let requested_materials: Vec<_> = self
            .editor
            .core
            .tex_registry
            .pending_requests
            .iter()
            .cloned()
            .collect();
        for material in requested_materials {
            self.editor
                .tex_browser
                .request_texture_load(&material, &self.editor.core.tex_registry);
        }

        let uploads_per_frame = self.editor.core.config.perf.tex_load_num;

        let batch = self
            .editor
            .tex_browser
            .drain_pending_uploads(uploads_per_frame);
        for (material, img) in batch {
            let tid = register_texture(&mut self.renderer, &img, "game texture");
            self.editor
                .tex_browser
                .tex_gpu_cache
                .insert(material, (tid, [img.width as f32, img.height as f32]));
        }
        let inserted_render_textures = self.editor.tex_browser.process_pending_render_uploads(
            &self.gl,
            uploads_per_frame,
            &self.editor.core.config.view.rendermode,
            upload_texture_mipmaps,
            &mut self.editor.core.tex_registry,
        );
        if inserted_render_textures {
            self.editor.core.view_config_rev = self.editor.core.view_config_rev.wrapping_add(1);
        }

        let lit_program = unsafe {
            let vert = self.gl.create_shader(glow::VERTEX_SHADER).unwrap();
            self.gl.shader_source(
                vert,
                include_str!("glsl/lit_vert.glsl"),
            );
            self.gl.compile_shader(vert);

            let frag = self.gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            self.gl.shader_source(
                frag,
                include_str!("glsl/lit_frag.glsl"),
            );
            self.gl.compile_shader(frag);

            let prog = self.gl.create_program().unwrap();
            self.gl.attach_shader(prog, vert);
            self.gl.attach_shader(prog, frag);
            self.gl.bind_attrib_location(prog, 0, "a_pos");
            self.gl.bind_attrib_location(prog, 1, "a_normal");
            self.gl.link_program(prog);
            prog
        };

        let lit_mvp_loc = unsafe { self.gl.get_uniform_location(lit_program, "u_mvp").unwrap() };
        let lit_color_loc = unsafe {
            self.gl
                .get_uniform_location(lit_program, "u_color")
                .unwrap()
        };
        let lit_ldir_loc = unsafe {
            self.gl
                .get_uniform_location(lit_program, "u_light_dir")
                .unwrap()
        };
        let lit_amb_loc = unsafe {
            self.gl
                .get_uniform_location(lit_program, "u_ambient")
                .unwrap()
        };

        let tex_program = unsafe {
            let vert = self.gl.create_shader(glow::VERTEX_SHADER).unwrap();
            self.gl.shader_source(
                vert,
                include_str!("glsl/tex_vert.glsl")
            );
            self.gl.compile_shader(vert);

            let frag = self.gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            self.gl.shader_source(
                frag,
                include_str!("glsl/tex_frag.glsl")
            );
            self.gl.compile_shader(frag);

            let prog = self.gl.create_program().unwrap();
            self.gl.attach_shader(prog, vert);
            self.gl.attach_shader(prog, frag);
            self.gl.bind_attrib_location(prog, 0, "a_pos");
            self.gl.bind_attrib_location(prog, 1, "a_normal");
            self.gl.bind_attrib_location(prog, 2, "a_uv");
            self.gl.link_program(prog);
            prog
        };

        let tex_mvp_loc = unsafe { self.gl.get_uniform_location(tex_program, "u_mvp").unwrap() };
        let tex_color_loc = unsafe {
            self.gl
                .get_uniform_location(tex_program, "u_color")
                .unwrap()
        };
        let tex_ldir_loc = unsafe {
            self.gl
                .get_uniform_location(tex_program, "u_light_dir")
                .unwrap()
        };
        let tex_amb_loc = unsafe {
            self.gl
                .get_uniform_location(tex_program, "u_ambient")
                .unwrap()
        };
        let tex_sampler_loc =
            unsafe { self.gl.get_uniform_location(tex_program, "u_tex").unwrap() };

        self.platform.prepare_frame(&self.window, &mut self.imgui);
        let ui = self.imgui.frame();
        ui::draw_editor(ui, &mut self.editor, delta);
        self.editor.sync_viewports_to_core();

        if self.editor.view3d.right_dragging {
            self.window.set_cursor_visible(false);
        } else {
            self.window.set_cursor_visible(true);
            self.editor.view3d.last_cursor_pos = None;
        }

        self.platform.prepare_render(&mut self.imgui, &self.window);
        let draw_data = self.imgui.render();

        //self.renderer.render(draw_data).expect("imgui render failed");

        unsafe {
            use glow::HasContext;
            let (win_w, win_h): (u32, u32) = self.window.inner_size().into();

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.viewport(0, 0, win_w as i32, win_h as i32);
            let win_clear = self.editor.core.palette.window_clear;
            self.gl
                .clear_color(win_clear[0], win_clear[1], win_clear[2], win_clear[3]);
            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            //self.gl.enable(glow::DEPTH_TEST);
            self.gl.clear(glow::DEPTH_BUFFER_BIT);

            let mut backend = RenderBackend {
                gl: &self.gl,
                wire_program: self.program,
                mvp_loc: self.mvp_loc.clone(),
                color_loc: self.color_loc.clone(),
                vao: self.vao,
                vbo: self.vbo,
                lit_program,
                lit_mvp_loc,
                lit_color_loc,
                lit_ldir_loc,
                lit_amb_loc,
                tex_program,
                tex_mvp_loc,
                tex_color_loc,
                tex_ldir_loc,
                tex_amb_loc,
                tex_sampler_loc,
            };
            self.vp3d.render(&mut backend, &mut self.editor.core);
            self.vp2d.render(&mut backend, &mut self.editor.core);

            self.gl.use_program(None);
        }

        let has_pending_texture_work = !self.editor.core.tex_registry.pending_requests.is_empty()
            || !self.editor.tex_browser.pending_uploads.is_empty()
            || !self.editor.tex_browser.pending_render_uploads.is_empty();

        self.renderer
            .render(draw_data)
            .expect("imgui render failed");
        self.gl_surface
            .swap_buffers(&self.gl_context)
            .expect("swap failed");

        self.needs_redraw = has_pending_texture_work;
        if has_pending_texture_work {
            self.window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            let mut state = AppState::new(event_loop);
            state.needs_redraw = true;
            state.window.request_redraw();
            self.state = Some(state);
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
                //let cfg = state.editor.config.clone();
                let _ = save_config(&state.editor.core.config);
                event_loop.exit()
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                state.gl_surface.resize(
                    &state.gl_context,
                    NonZeroU32::new(size.width).unwrap(),
                    NonZeroU32::new(size.height).unwrap(),
                );
                state.needs_redraw = true;
                state.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                if state.ignore_next_cursor_move {
                    state.ignore_next_cursor_move = false;
                    state.editor.view3d.last_cursor_pos =
                        Some([position.x as f32, position.y as f32]);
                    state.needs_redraw = true;
                    state.window.request_redraw();
                    return;
                }

                if state.editor.view3d.right_dragging {
                    if let Some([last_x, last_y]) = state.editor.view3d.last_cursor_pos {
                        state.editor.view3d.accumulated_mouse_delta[0] +=
                            position.x as f32 - last_x;
                        state.editor.view3d.accumulated_mouse_delta[1] +=
                            position.y as f32 - last_y;
                    }

                    let rect = state.editor.view3d.rect;
                    let left = rect[0] as f64;
                    let top = rect[1] as f64;
                    let right = (rect[0] + rect[2]) as f64;
                    let bottom = (rect[1] + rect[3]) as f64;
                    let mut warp_x = position.x;
                    let mut warp_y = position.y;
                    let pad = 24.0f64;

                    if position.x <= left {
                        warp_x = right - pad;
                    } else if position.x >= right {
                        warp_x = left + pad;
                    }

                    if position.y <= top {
                        warp_y = bottom - pad;
                    } else if position.y >= bottom {
                        warp_y = top + pad;
                    }

                    if (warp_x - position.x).abs() > f64::EPSILON
                        || (warp_y - position.y).abs() > f64::EPSILON
                    {
                        state.ignore_next_cursor_move = true;
                        let _ = state
                            .window
                            .set_cursor_position(PhysicalPosition::new(warp_x, warp_y));
                        state.editor.view3d.last_cursor_pos =
                            Some([warp_x as f32, warp_y as f32]);
                    } else {
                        state.editor.view3d.last_cursor_pos =
                            Some([position.x as f32, position.y as f32]);
                    }
                }
                state.needs_redraw = true;
                state.window.request_redraw();
            }
            WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. } => {
                state.needs_redraw = true;
                state.window.request_redraw();
            }
            // Any input/UI event should schedule a redraw; we avoid continuous rendering when idle
            WindowEvent::MouseInput {
                button: winit::event::MouseButton::Right,
                state: winit::event::ElementState::Released,
                ..
            } => {
                state.ignore_next_cursor_move = false;
                state.needs_redraw = true;
                state.render();
                state.needs_redraw = false;
            }
            WindowEvent::MouseInput { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_) => {
                state.needs_redraw = true;
                state.render();
                state.needs_redraw = false;
            }
            WindowEvent::Focused(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_) => {
                state.needs_redraw = true;
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if state.needs_redraw {
                    state.render();
                    state.needs_redraw = false;
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            if state.needs_redraw {
                state.window.request_redraw(); // keep drawing while UI is active
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let Some(state) = &mut self.state {
            if let winit::event::DeviceEvent::MouseMotion { delta } = event {
                if state.editor.view3d.right_dragging {
                    state.editor.view3d.accumulated_mouse_delta[0] += delta.0 as f32;
                    state.editor.view3d.accumulated_mouse_delta[1] += delta.1 as f32;
                    state.needs_redraw = true;
                    state.window.request_redraw();
                }
            }
        }
    }
}
