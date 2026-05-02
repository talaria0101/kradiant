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
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::config::EditorConfig;
use crate::icons::EditorIcons;
use crate::images::EditorImages;
use crate::viewport2d::Viewport2D;
use crate::viewport3d::{LitVertex, TexVertex, Viewport3D};

#[macro_use]
mod ui;
mod config;
mod icons;
mod images;
mod theme;
mod util;

mod viewport2d;
mod viewport3d;

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
}

pub struct RenderBackend<'a> {
    pub gl: &'a glow::Context,
    pub wire_program: glow::Program,
    pub mvp_loc: glow::UniformLocation,
    pub color_loc: glow::UniformLocation,
    pub vao: glow::NativeVertexArray,
    pub vbo: glow::Buffer,
    pub lit_program: glow::NativeProgram,
    pub lit_mvp_loc: glow::NativeUniformLocation,
    pub lit_color_loc: glow::NativeUniformLocation,
    pub lit_ldir_loc: glow::NativeUniformLocation,
    pub lit_amb_loc: glow::NativeUniformLocation,
    pub tex_program: glow::NativeProgram,
    pub tex_mvp_loc: glow::NativeUniformLocation,
    pub tex_color_loc: glow::NativeUniformLocation,
    pub tex_ldir_loc: glow::NativeUniformLocation,
    pub tex_amb_loc: glow::NativeUniformLocation,
    pub tex_sampler_loc: glow::NativeUniformLocation,
}

impl RenderBackend<'_> {
    pub unsafe fn draw_lines(&self, vertices: &[Vec3], color: [f32; 4], mvp: glam::Mat4) {
        unsafe {
            if vertices.is_empty() {
                return;
            }
            self.gl.use_program(Some(self.wire_program));
            self.gl
                .uniform_matrix_4_f32_slice(Some(&self.mvp_loc), false, &mvp.to_cols_array());
            self.gl.uniform_4_f32_slice(Some(&self.color_loc), &color);
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );
            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
            self.gl.enable_vertex_attrib_array(0);
            self.gl.draw_arrays(glow::LINES, 0, vertices.len() as i32);
        }
    }

    /*pub unsafe fn draw_triangles(
        &self,
        vertices: &[Vec3],
        color: [f32; 4],
        mvp: glam::Mat4,
    ) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.wire_program)); // fine for now
            self.gl
            .uniform_matrix_4_f32_slice(Some(&self.mvp_loc), false, &mvp.to_cols_array());
            self.gl.uniform_4_f32_slice(Some(&self.color_loc), &color);

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                                         glow::STREAM_DRAW,
            );

            self.gl
            .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
            self.gl.enable_vertex_attrib_array(0);

            self.gl.draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);
        }
    }*/

    pub unsafe fn draw_triangles_lit(
        &self,
        vertices: &[LitVertex],
        color: [f32; 4],
        mvp: glam::Mat4,
        ambient: f32,
        light_dir: Vec3,
    ) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.lit_program));
            self.gl.uniform_matrix_4_f32_slice(
                Some(&self.lit_mvp_loc),
                false,
                &mvp.to_cols_array(),
            );
            self.gl
                .uniform_4_f32_slice(Some(&self.lit_color_loc), &color);
            self.gl.uniform_3_f32(
                Some(&self.lit_ldir_loc),
                light_dir.x,
                light_dir.y,
                light_dir.z,
            );
            self.gl.uniform_1_f32(Some(&self.lit_amb_loc), ambient);

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );

            // stride = 24, pos at offset 0, normal at offset 12
            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 24, 0);
            self.gl.enable_vertex_attrib_array(0);
            self.gl
                .vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 24, 12);
            self.gl.enable_vertex_attrib_array(1);

            self.gl
                .draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);

            self.gl.disable_vertex_attrib_array(1); // don't leave attrib 1 enabled for wire draws
        }
    }

    pub unsafe fn draw_triangles_tex(
        &self,
        vertices: &[TexVertex],
        texture: glow::Texture,
        color: [f32; 4],
        mvp: glam::Mat4,
        ambient: f32,
        light_dir: Vec3,
    ) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.tex_program));

            self.gl.uniform_matrix_4_f32_slice(
                Some(&self.tex_mvp_loc),
                false,
                &mvp.to_cols_array(),
            );
            self.gl
                .uniform_4_f32_slice(Some(&self.tex_color_loc), &color);
            self.gl.uniform_3_f32(
                Some(&self.tex_ldir_loc),
                light_dir.x,
                light_dir.y,
                light_dir.z,
            );
            self.gl.uniform_1_f32(Some(&self.tex_amb_loc), ambient);
            self.gl.uniform_1_i32(Some(&self.tex_sampler_loc), 0);

            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );

            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 32, 0);
            self.gl.enable_vertex_attrib_array(0);

            // Normal at offset 12
            self.gl
                .vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 32, 12);
            self.gl.enable_vertex_attrib_array(1);

            // UV at offset 24
            self.gl
                .vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, 32, 24);
            self.gl.enable_vertex_attrib_array(2);

            self.gl
                .draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);

            // Cleanup
            self.gl.disable_vertex_attrib_array(2);
            self.gl.disable_vertex_attrib_array(1);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
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
/// Some todos:
///
/// TODO: Alternative filtering options
///
/// Filtering modes explained:
///
/// - GL_LINEAR_MIPMAP_LINEAR: Trilinear filtering—interpolates between 2 mipmap levels, each bilinearly filtered (best quality)
/// - GL_LINEAR_MIPMAP_NEAREST: Bilinear within nearest mipmap level (faster, lower quality)
/// - GL_NEAREST_MIPMAP_LINEAR: Linear interpolation between mipmap levels, nearest within each
/// - GL_NEAREST_MIPMAP_NEAREST: Nearest mipmap level, nearest pixel (fastest, pixelated)
///
/// TODO: Togglable repeat mode
unsafe fn upload_texture_mipmaps(gl: &glow::Context, size: [u32; 2], rgba: &[u8]) -> glow::Texture {
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
            r#"
        #version 120
        attribute vec3 a_pos;
        uniform mat4 u_mvp;
        void main() { gl_Position = u_mvp * vec4(a_pos, 1.0); }
        "#,
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
            r#"
        #version 120
        uniform vec4 u_color;
        void main() { gl_FragColor = u_color; }
        "#,
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
        if !editor.config.paths.main.as_os_str().is_empty() {
            editor.tex_browser.init(&editor.config.paths.main);
            editor.console.info("Asset database loaded".to_string());
        }

        // Apply configured theme on startup
        if !editor.themes.is_empty() {
            let idx = editor
                .config
                .misc
                .theme
                .min(editor.themes.len().saturating_sub(1));
            if let Some(entry) = editor.themes.get(idx) {
                theme::apply_theme(&mut imgui, &entry.data);
                EditorConfig::update(&mut editor.config, "active_theme", idx, &mut editor.console);
            }
        }
        editor.palette = theme::palette_from_theme(
            &imgui,
            editor.themes.get(editor.config.misc.theme).map(|e| &e.data),
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
            view3d_fbo,
            view3d_fbo_size,
            view3d_tex,
            view3d_rbo,
            None,
        );

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
                EditorConfig::update(
                    &mut self.editor.config,
                    "active_theme",
                    idx,
                    &mut self.editor.console,
                );
            }
        }

        self.editor.palette = theme::palette_from_theme(
            &self.imgui,
            self.editor
                .themes
                .get(self.editor.config.misc.theme)
                .map(|e| &e.data),
        );

        const UPLOADS_PER_FRAME: usize = 4; // later would add in configuration
        let pending = &mut self.editor.tex_browser.pending_uploads;
        let batch: Vec<_> = pending
            .drain(..pending.len().min(UPLOADS_PER_FRAME))
            .collect();
        for (material, img) in batch {
            let tid = register_texture(&mut self.renderer, &img, "game texture");
            self.editor
                .tex_browser
                .tex_gpu_cache
                .insert(material, (tid, [img.width as f32, img.height as f32]));
        }
        self.editor.tex_browser.process_pending_render_uploads(
            &self.gl,
            UPLOADS_PER_FRAME,
            upload_texture_mipmaps,
        );

        let lit_program = unsafe {
            let vert = self.gl.create_shader(glow::VERTEX_SHADER).unwrap();
            self.gl.shader_source(
                vert,
                r#"
            #version 120
            attribute vec3 a_pos;
            attribute vec3 a_normal;
            uniform mat4 u_mvp;
            varying vec3 v_normal;
            void main() {
            gl_Position = u_mvp * vec4(a_pos, 1.0);
            v_normal = a_normal;
        }
        "#,
            );
            self.gl.compile_shader(vert);

            let frag = self.gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            self.gl.shader_source(
                frag,
                r#"
            #version 120
            uniform vec4 u_color;
            uniform vec3 u_light_dir;   // normalized, world space
            uniform float u_ambient;
            varying vec3 v_normal;
            void main() {
            vec3 n = normalize(v_normal);
            // Sample both sides so back-faces aren't black
            float d = max(dot(n, u_light_dir), max(dot(-n, u_light_dir), 0.0));
            float light = u_ambient + (1.0 - u_ambient) * d;
            gl_FragColor = vec4(u_color.rgb * light, u_color.a);
        }
        "#,
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
                r#"
                #version 120
                attribute vec3 a_pos;
                attribute vec3 a_normal;
                attribute vec2 a_uv;        // New: texture coordinates
                uniform mat4 u_mvp;
                varying vec3 v_normal;
                varying vec2 v_uv;          // Pass to fragment shader
                void main() {
                    gl_Position = u_mvp * vec4(a_pos, 1.0);
                    v_normal = a_normal;
                    v_uv = a_uv;
                }
                "#,
            );
            self.gl.compile_shader(vert);

            let frag = self.gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            self.gl.shader_source(
                frag,
                r#"
                #version 120
                uniform sampler2D u_tex;  // Texture sampler
                uniform vec4 u_color;
                uniform vec3 u_light_dir;
                uniform float u_ambient;
                varying vec3 v_normal;
                varying vec2 v_uv;
                void main() {
                    vec3 n = normalize(v_normal);
                    float d = max(dot(n, u_light_dir), max(dot(-n, u_light_dir), 0.0));
                    float light = u_ambient + (1.0 - u_ambient) * d;

                    vec4 tex_color = texture2D(u_tex, v_uv);  // Sample texture
                    gl_FragColor = vec4(tex_color.rgb * u_color.rgb * light, tex_color.a * u_color.a);
                }
                "#
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

        if let Some(warp) = self.editor.view3d.warp_request.take() {
            let fb_scale = self.imgui.io().display_framebuffer_scale();
            let sx = fb_scale[0].max(1.0) as f64;
            let sy = fb_scale[1].max(1.0) as f64;
            let _ = self
                .window
                .set_cursor_position(winit::dpi::PhysicalPosition::new(
                    warp[0] as f64 * sx,
                    warp[1] as f64 * sy,
                ));
            self.editor.view3d.warp_pending_reset = true;
        }

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
            self.vp3d.render(&mut backend, &mut self.editor);
            self.vp2d.render(&mut backend, &mut self.editor);

            self.gl.use_program(None);
        }

        self.renderer
            .render(draw_data)
            .expect("imgui render failed");
        self.gl_surface
            .swap_buffers(&self.gl_context)
            .expect("swap failed");

        self.needs_redraw = false;
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
                let _ = state.editor.config.save(); //EditorConfig::save(cfg);
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
            WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. } => {
                state.needs_redraw = true;
                state.window.request_redraw();
            }
            // Any input/UI event should schedule a redraw; we avoid continuous rendering when idle
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
}
