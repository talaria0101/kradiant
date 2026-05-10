use dear_imgui_rs::StyleColor;
use kradiant::editor::theme::EditorPalette;
use serde::Deserialize;
use std::collections::HashMap;

pub struct ThemeEntry {
    pub name: String,
    pub data: ThemeFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFile {
    pub alpha: Option<f32>,
    pub disabled_alpha: Option<f32>,

    pub window_padding: Option<[f32; 2]>,
    pub window_rounding: Option<f32>,
    pub window_border_size: Option<f32>,

    pub child_rounding: Option<f32>,
    pub child_border_size: Option<f32>,

    pub popup_rounding: Option<f32>,
    pub popup_border_size: Option<f32>,

    pub frame_padding: Option<[f32; 2]>,
    pub frame_rounding: Option<f32>,
    pub frame_border_size: Option<f32>,

    pub item_spacing: Option<[f32; 2]>,
    pub item_inner_spacing: Option<[f32; 2]>,

    pub indent_spacing: Option<f32>,
    pub scrollbar_size: Option<f32>,
    pub scrollbar_rounding: Option<f32>,

    pub grab_min_size: Option<f32>,
    pub grab_rounding: Option<f32>,

    pub tab_rounding: Option<f32>,

    pub colors: Option<HashMap<String, String>>,

    pub editor: Option<EditorThemeFile>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EditorThemeFile {
    pub window_clear: Option<String>,
    pub view2d_bg: Option<String>,
    pub view2d_grid_major: Option<String>,
    pub view2d_grid_minor: Option<String>,
    pub view2d_axis_x: Option<String>,
    pub view2d_axis_y: Option<String>,
    pub view2d_geometry: Option<String>,
    pub console_info: Option<String>,
    pub console_warn: Option<String>,
    pub console_error: Option<String>,
    pub hud_text: Option<String>,
    pub hud_text_dim: Option<String>,
}

pub fn parse_rgba(s: &str) -> Option<[f32; 4]> {
    let s = s.trim().strip_prefix("rgba(")?.strip_suffix(')')?.trim();

    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != 4 {
        return None;
    }

    let r = parts[0].parse::<f32>().ok()? / 255.0;
    let g = parts[1].parse::<f32>().ok()? / 255.0;
    let b = parts[2].parse::<f32>().ok()? / 255.0;
    let a = parts[3].parse::<f32>().ok()?;

    Some([r, g, b, a])
}

pub fn theme_from_str(src: &str) -> Result<ThemeFile, toml::de::Error> {
    toml::from_str(src)
}

pub fn palette_from_theme(
    imgui: &dear_imgui_rs::Context,
    theme: Option<&ThemeFile>,
) -> EditorPalette {
    let style = imgui.style();

    // Prefer ChildBg as the "canvas" background if it isn't fully transparent.
    let window_bg = style.color(StyleColor::WindowBg);
    let child_bg = style.color(StyleColor::ChildBg);
    let view_bg = if child_bg[3] > 0.01 {
        child_bg
    } else {
        window_bg
    };
    let text = style.color(StyleColor::Text);
    let text_disabled = style.color(StyleColor::TextDisabled);

    let mut palette = EditorPalette {
        // Default to ImGui style colors (so themes that omit `[editor]` still look coherent),
        // but prefer putting explicit editor colors in the theme TOML `[editor]` table.
        window_clear: [window_bg[0], window_bg[1], window_bg[2], 1.0],
        view2d_bg: [view_bg[0], view_bg[1], view_bg[2], 1.0],
        view2d_grid_major: {
            let c = style.color(StyleColor::Border);
            [c[0], c[1], c[2], 1.0]
        },
        view2d_grid_minor: {
            let c = style.color(StyleColor::FrameBg);
            [c[0], c[1], c[2], 1.0]
        },
        view2d_axis_x: {
            let c = style.color(StyleColor::TabSelectedOverline);
            [c[0], c[1], c[2], 1.0]
        },
        view2d_axis_y: {
            let c = style.color(StyleColor::CheckMark);
            [c[0], c[1], c[2], 1.0]
        },
        view2d_geometry: [text[0], text[1], text[2], 1.0],
        console_info: text,
        console_warn: style.color(StyleColor::PlotHistogramHovered),
        console_error: style.color(StyleColor::DragDropTarget),
        hud_text: text,
        hud_text_dim: text_disabled,
    };

    // Optional explicit overrides from theme file.
    if let Some(theme) = theme {
        if let Some(editor) = &theme.editor {
            let override_color = |dst: &mut [f32; 4], src: &Option<String>| {
                if let Some(s) = src.as_deref() {
                    if let Some(c) = parse_rgba(s) {
                        *dst = c;
                    }
                }
            };
            override_color(&mut palette.window_clear, &editor.window_clear);
            override_color(&mut palette.view2d_bg, &editor.view2d_bg);
            override_color(&mut palette.view2d_grid_major, &editor.view2d_grid_major);
            override_color(&mut palette.view2d_grid_minor, &editor.view2d_grid_minor);
            override_color(&mut palette.view2d_axis_x, &editor.view2d_axis_x);
            override_color(&mut palette.view2d_axis_y, &editor.view2d_axis_y);
            override_color(&mut palette.view2d_geometry, &editor.view2d_geometry);
            override_color(&mut palette.console_info, &editor.console_info);
            override_color(&mut palette.console_warn, &editor.console_warn);
            override_color(&mut palette.console_error, &editor.console_error);
            override_color(&mut palette.hud_text, &editor.hud_text);
            override_color(&mut palette.hud_text_dim, &editor.hud_text_dim);
        }
    }

    palette
}

pub fn apply_theme(imgui: &mut dear_imgui_rs::Context, theme: &ThemeFile) {
    let style = imgui.style_mut();

    if let Some(v) = theme.alpha {
        style.set_alpha(v);
    }
    if let Some(v) = theme.disabled_alpha {
        style.set_disabled_alpha(v);
    }

    if let Some(v) = theme.window_padding {
        style.set_window_padding(v);
    }
    if let Some(v) = theme.window_rounding {
        style.set_window_rounding(v);
    }
    if let Some(v) = theme.window_border_size {
        style.set_window_border_size(v);
    }

    if let Some(v) = theme.child_rounding {
        style.set_child_rounding(v);
    }
    if let Some(v) = theme.child_border_size {
        style.set_child_border_size(v);
    }

    if let Some(v) = theme.popup_rounding {
        style.set_popup_rounding(v);
    }
    if let Some(v) = theme.popup_border_size {
        style.set_popup_border_size(v);
    }

    if let Some(v) = theme.frame_padding {
        style.set_frame_padding(v);
    }
    if let Some(v) = theme.frame_rounding {
        style.set_frame_rounding(v);
    }
    if let Some(v) = theme.frame_border_size {
        style.set_frame_border_size(v);
    }

    if let Some(v) = theme.item_spacing {
        style.set_item_spacing(v);
    }
    if let Some(v) = theme.item_inner_spacing {
        style.set_item_inner_spacing(v);
    }

    if let Some(v) = theme.indent_spacing {
        style.set_indent_spacing(v);
    }
    if let Some(v) = theme.scrollbar_size {
        style.set_scrollbar_size(v);
    }
    if let Some(v) = theme.scrollbar_rounding {
        style.set_scrollbar_rounding(v);
    }

    if let Some(v) = theme.grab_min_size {
        style.set_grab_min_size(v);
    }
    if let Some(v) = theme.grab_rounding {
        style.set_grab_rounding(v);
    }
    if let Some(v) = theme.tab_rounding {
        style.set_tab_rounding(v);
    }

    // Colors
    if let Some(colors) = &theme.colors {
        let mapping: &[(&str, StyleColor)] = &[
            ("Text", StyleColor::Text),
            ("TextDisabled", StyleColor::TextDisabled),
            ("WindowBg", StyleColor::WindowBg),
            ("ChildBg", StyleColor::ChildBg),
            ("PopupBg", StyleColor::PopupBg),
            ("Border", StyleColor::Border),
            ("BorderShadow", StyleColor::BorderShadow),
            ("FrameBg", StyleColor::FrameBg),
            ("FrameBgHovered", StyleColor::FrameBgHovered),
            ("FrameBgActive", StyleColor::FrameBgActive),
            ("TitleBg", StyleColor::TitleBg),
            ("TitleBgActive", StyleColor::TitleBgActive),
            ("TitleBgCollapsed", StyleColor::TitleBgCollapsed),
            ("MenuBarBg", StyleColor::MenuBarBg),
            ("ScrollbarBg", StyleColor::ScrollbarBg),
            ("ScrollbarGrab", StyleColor::ScrollbarGrab),
            ("ScrollbarGrabHovered", StyleColor::ScrollbarGrabHovered),
            ("ScrollbarGrabActive", StyleColor::ScrollbarGrabActive),
            ("CheckMark", StyleColor::CheckMark),
            ("SliderGrab", StyleColor::SliderGrab),
            ("SliderGrabActive", StyleColor::SliderGrabActive),
            ("Button", StyleColor::Button),
            ("ButtonHovered", StyleColor::ButtonHovered),
            ("ButtonActive", StyleColor::ButtonActive),
            ("Header", StyleColor::Header),
            ("HeaderHovered", StyleColor::HeaderHovered),
            ("HeaderActive", StyleColor::HeaderActive),
            ("Separator", StyleColor::Separator),
            ("SeparatorHovered", StyleColor::SeparatorHovered),
            ("SeparatorActive", StyleColor::SeparatorActive),
            ("ResizeGrip", StyleColor::ResizeGrip),
            ("ResizeGripHovered", StyleColor::ResizeGripHovered),
            ("ResizeGripActive", StyleColor::ResizeGripActive),
            ("Tab", StyleColor::Tab),
            ("TabHovered", StyleColor::TabHovered),
            ("TabActive", StyleColor::TabSelected),
            ("TabActive", StyleColor::TabSelectedOverline),
            ("TabUnfocused", StyleColor::TabDimmed),
            ("TabUnfocusedActive", StyleColor::TabDimmedSelected),
            ("TabUnfocusedActive", StyleColor::TabDimmedSelectedOverline),
            ("PlotLines", StyleColor::PlotLines),
            ("PlotLinesHovered", StyleColor::PlotLinesHovered),
            ("PlotHistogram", StyleColor::PlotHistogram),
            ("PlotHistogramHovered", StyleColor::PlotHistogramHovered),
            ("TableHeaderBg", StyleColor::TableHeaderBg),
            ("TableBorderStrong", StyleColor::TableBorderStrong),
            ("TableBorderLight", StyleColor::TableBorderLight),
            ("TableRowBg", StyleColor::TableRowBg),
            ("TableRowBgAlt", StyleColor::TableRowBgAlt),
            ("TextSelectedBg", StyleColor::TextSelectedBg),
            ("DragDropTarget", StyleColor::DragDropTarget),
            ("NavHighlight", StyleColor::NavCursor),
            ("NavWindowingHighlight", StyleColor::NavWindowingHighlight),
            ("NavWindowingDimBg", StyleColor::NavWindowingDimBg),
            ("ModalWindowDimBg", StyleColor::ModalWindowDimBg),
        ];

        for (name, variant) in mapping {
            if let Some(rgba_str) = colors.get(*name) {
                if let Some(col) = parse_rgba(rgba_str) {
                    style.set_color(*variant, col);
                }
            }
        }
    }
}
