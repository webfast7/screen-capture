use std::{fs, path::PathBuf};

use ab_glyph::FontArc;
use capture_core::{CaptureError, CapturedImage};
use eframe::egui::{
    self, Align, Color32, ColorImage, Context, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Id, Layout, Margin, Pos2, Rect, RichText, Rounding, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2, Visuals,
};
use image::{ColorType, ImageEncoder as _, Rgba, RgbaImage, codecs::png::PngEncoder};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_line_segment_mut, draw_text_mut},
    rect::Rect as ImageRect,
};
use capture_platform_linux::{BackendSelection, copy_png};
use tracing::{error, info};

const PRESET_COLORS: [(&str, Color32); 8] = [
    ("Red", Color32::from_rgb(255, 59, 48)),
    ("Orange", Color32::from_rgb(255, 149, 0)),
    ("Yellow", Color32::from_rgb(255, 204, 0)),
    ("Green", Color32::from_rgb(52, 199, 89)),
    ("Blue", Color32::from_rgb(0, 122, 255)),
    ("Purple", Color32::from_rgb(175, 82, 222)),
    ("Black", Color32::from_rgb(28, 28, 30)),
    ("White", Color32::from_rgb(255, 255, 255)),
];

#[derive(Debug, Clone)]
pub struct EditorBootstrap {
    image: CapturedImage,
    font_path: Option<PathBuf>,
}

impl EditorBootstrap {
    pub fn new(image: CapturedImage, font_path: Option<PathBuf>) -> Self {
        Self { image, font_path }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Rectangle,
    Arrow,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ImagePoint {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone)]
enum Annotation {
    Rectangle {
        from: ImagePoint,
        to: ImagePoint,
        color: Color32,
        stroke_width: u32,
    },
    Arrow {
        from: ImagePoint,
        to: ImagePoint,
        color: Color32,
        stroke_width: u32,
    },
    Text {
        at: ImagePoint,
        text: String,
        color: Color32,
        size: f32,
    },
}

#[derive(Debug, Clone)]
struct DragState {
    start: ImagePoint,
    current: ImagePoint,
}

#[derive(Debug, Clone)]
enum CanvasDrag {
    NewRectangle(DragState),
    NewArrow(DragState),
    MoveAnnotation { index: usize, grab_offset: ImagePoint },
}

#[derive(Debug, Clone)]
struct TextDraft {
    at: ImagePoint,
    screen_pos: Pos2,
    text: String,
    edit_index: Option<usize>,
}

pub struct AnnotationEditorApp {
    base_image: CapturedImage,
    output_path: PathBuf,
    annotations: Vec<Annotation>,
    texture: Option<TextureHandle>,
    canvas_drag: Option<CanvasDrag>,
    text_draft: Option<TextDraft>,
    selected_annotation: Option<usize>,
    tool: Tool,
    color: Color32,
    stroke_width: u32,
    text_size: f32,
    status: String,
    output_feedback: Option<String>,
    font_path: Option<PathBuf>,
}

impl AnnotationEditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, bootstrap: EditorBootstrap) -> Self {
        configure_visuals(&cc.egui_ctx);
        configure_fonts(&cc.egui_ctx, bootstrap.font_path.as_ref());

        let output_path = capture_utils::build_output_path(
            None,
            capture_utils::default_filename(capture_core::ImageFormat::Png),
        );
        let mut app = Self {
            base_image: bootstrap.image,
            output_path,
            annotations: Vec::new(),
            texture: None,
            canvas_drag: None,
            text_draft: None,
            selected_annotation: None,
            tool: Tool::Rectangle,
            color: Color32::from_rgb(255, 59, 48),
            stroke_width: 3,
            text_size: 28.0,
            status: "当前为矩形工具，可在图片上拖拽创建标注框。".to_string(),
            output_feedback: None,
            font_path: bootstrap.font_path,
        };
        app.ensure_texture(&cc.egui_ctx);
        app
    }

    fn ensure_texture(&mut self, ctx: &Context) {
        if self.texture.is_some() {
            return;
        }

        let color_image = ColorImage::from_rgba_unmultiplied(
            [
                self.base_image.resolution.width as usize,
                self.base_image.resolution.height as usize,
            ],
            &self.base_image.pixels,
        );
        self.texture = Some(ctx.load_texture(
            "capture-editor-image",
            color_image,
            TextureOptions::LINEAR,
        ));
    }

    fn save_annotated_image(&mut self) {
        match self.render_annotated_image().and_then(|image| {
            image
                .save(&self.output_path)
                .map_err(|error| CaptureError::Output {
                    message: format!("failed to save annotated image: {error}"),
                })
        }) {
            Ok(()) => {
                let detail = format!("已保存标注截图到 {}", self.output_path.display());
                info!(path = %self.output_path.display(), "editor: save finished");
                self.status = detail.clone();
                self.output_feedback = Some(detail);
            }
            Err(error) => {
                error!(error = %error, "editor: save failed");
                self.status = error.to_string();
                self.output_feedback = Some(self.status.clone());
            }
        }
    }

    fn copy_annotated_image(&mut self) {
        match self
            .render_annotated_image()
            .and_then(|image| encode_png(&image))
            .and_then(|bytes| {
                let backend = copy_png(&bytes, BackendSelection::Auto)?;
                Ok(backend)
            }) {
            Ok(backend) => {
                let detail = format!("已通过 {} 复制标注截图到剪贴板", backend.name());
                info!(clipboard = backend.name(), "editor: clipboard copy finished");
                self.status = detail.clone();
                self.output_feedback = Some(detail);
            }
            Err(error) => {
                error!(error = %error, "editor: clipboard copy failed");
                self.status = error.to_string();
                self.output_feedback = Some(self.status.clone());
            }
        }
    }

    fn render_annotated_image(&self) -> Result<RgbaImage, CaptureError> {
        let mut image = RgbaImage::from_raw(
            self.base_image.resolution.width,
            self.base_image.resolution.height,
            self.base_image.pixels.clone(),
        )
        .ok_or_else(|| CaptureError::InvalidImageBuffer {
            expected: self.base_image.pixels.len(),
            actual: 0,
        })?;

        let font = load_font(self.font_path.as_ref())?;
        for annotation in &self.annotations {
            match annotation {
                Annotation::Rectangle {
                    from,
                    to,
                    color,
                    stroke_width,
                } => {
                    let bounds = normalized_bounds(*from, *to);
                    let rect = ImageRect::at(bounds.min_x as i32, bounds.min_y as i32)
                        .of_size(bounds.width.max(1), bounds.height.max(1));
                    for offset in 0..*stroke_width {
                        let inset = offset as i32;
                        let width = bounds.width.saturating_sub(offset * 2).max(1);
                        let height = bounds.height.saturating_sub(offset * 2).max(1);
                        draw_hollow_rect_mut(
                            &mut image,
                            ImageRect::at(bounds.min_x as i32 + inset, bounds.min_y as i32 + inset)
                                .of_size(width, height),
                            rgba(*color),
                        );
                    }
                    let _ = rect;
                }
                Annotation::Text {
                    at,
                    text,
                    color,
                    size,
                } => {
                    let bounds = text_bounds(*at, text, *size);
                    draw_filled_rect_mut(
                        &mut image,
                        ImageRect::at(bounds.min_x as i32, bounds.min_y as i32)
                            .of_size(bounds.width.max(1), bounds.height.max(1)),
                        rgba(text_background(*color)),
                    );
                    draw_text_mut(
                        &mut image,
                        rgba(*color),
                        at.x.round() as i32,
                        at.y.round() as i32,
                        *size,
                        &font,
                        text,
                    );
                }
                Annotation::Arrow {
                    from,
                    to,
                    color,
                    stroke_width,
                } => {
                    draw_arrow_mut(&mut image, *from, *to, *color, *stroke_width);
                }
            }
        }

        Ok(image)
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("截图标注编辑器")
                        .text_style(egui::TextStyle::Heading)
                        .strong()
                        .color(Color32::from_rgb(245, 247, 250)),
                );
                ui.label(
                    RichText::new("对截图进行标注，然后保存或复制。")
                        .color(Color32::from_gray(170)),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_sized(
                        [120.0, 36.0],
                        egui::Button::new(RichText::new("保存 PNG").strong())
                            .fill(Color32::from_rgb(24, 119, 242)),
                    )
                    .clicked()
                {
                    self.save_annotated_image();
                }
                if ui
                    .add_sized(
                        [120.0, 36.0],
                        egui::Button::new("复制 PNG").fill(Color32::from_rgb(44, 48, 58)),
                    )
                    .clicked()
                {
                    self.copy_annotated_image();
                }
                if ui
                    .add_sized(
                        [96.0, 36.0],
                        egui::Button::new("撤销").fill(Color32::from_rgb(44, 48, 58)),
                    )
                    .clicked()
                {
                    self.undo_last_annotation();
                }
            });
        });
    }

    fn draw_side_panel(&mut self, ui: &mut egui::Ui) {
        section_card(ui, "工具", |ui| {
            ui.horizontal(|ui| {
                tool_chip(ui, &mut self.tool, Tool::Rectangle, "矩形");
                tool_chip(ui, &mut self.tool, Tool::Arrow, "箭头");
                tool_chip(ui, &mut self.tool, Tool::Text, "文字");
            });
        });

        section_card(ui, "样式", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("颜色").color(Color32::from_gray(190)));
                color_menu_button(ui, &mut self.color);
            });
            ui.add_space(8.0);
            ui.add(egui::Slider::new(&mut self.stroke_width, 1..=12).text("线宽"));
            ui.add(egui::Slider::new(&mut self.text_size, 16.0..=72.0).text("字号"));
        });

        section_card(ui, "提示", |ui| {
            let mode_hint = match self.tool {
                Tool::Rectangle => "在图片上拖拽即可创建矩形标注。",
                Tool::Arrow => "在图片上拖拽即可创建箭头标注。",
                Tool::Text => "点击图片即可在当前位置输入文字。",
            };
            ui.label(mode_hint);
            ui.add_space(8.0);
            ui.label("快捷键");
            ui.label("Ctrl+Z：撤销");
            ui.label("Ctrl+S：保存");
            ui.label("Ctrl+Shift+C：复制");
        });

        let font_label = self
            .font_path
            .as_ref()
            .map(|path| format!("字体：{}", path.display()))
            .unwrap_or_else(|| "字体：不可用".to_string());
        section_card(ui, "当前会话", |ui| {
            ui.label(format!("标注数量：{}", self.annotations.len()));
            ui.label(font_label);
            if let Some(index) = self.selected_annotation {
                if let Some(Annotation::Text { .. }) = self.annotations.get(index) {
                    ui.add_space(8.0);
                    if ui.button("编辑选中文字").clicked() {
                        self.start_text_edit(index);
                    }
                }
            }
        });
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        let font_label = self
            .font_path
            .as_ref()
            .map(|path| format!("字体：{}", path.display()))
            .unwrap_or_else(|| "字体：不可用，文字导出可能失败".to_string());
        ui.label(font_label);
        let mode_hint = match self.tool {
            Tool::Rectangle => "矩形模式：拖拽即可绘制标注框。",
            Tool::Arrow => "箭头模式：拖拽即可绘制箭头。",
            Tool::Text => "文字模式：点击图片后输入内容。",
        };
        ui.label(mode_hint);
        ui.label("快捷键：Ctrl+Z 撤销，Ctrl+S 保存，Ctrl+Shift+C 复制");
    }

    fn draw_image_canvas(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let texture = match &self.texture {
            Some(texture) => texture,
            None => return,
        };

        let image_size = Vec2::new(
            self.base_image.resolution.width as f32,
            self.base_image.resolution.height as f32,
        );
        let response = ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(image_size)
                .sense(Sense::click_and_drag()),
        );
        let image_rect = response.rect;
        if let Some(point) = response
            .hover_pos()
            .and_then(|pos| image_pos_from_screen(pos, image_rect, image_size))
        {
            if self.hit_test_annotation(point).is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Move);
            }
        }
        if matches!(self.canvas_drag, Some(CanvasDrag::MoveAnnotation { .. })) {
            ctx.set_cursor_icon(egui::CursorIcon::Move);
        }
        if response.drag_started() {
            if let Some(point) = response
                .interact_pointer_pos()
                .and_then(|pos| image_pos_from_screen(pos, image_rect, image_size))
            {
                if let Some(index) = self.hit_test_annotation(point) {
                    self.selected_annotation = Some(index);
                    let anchor = annotation_anchor(&self.annotations[index]);
                    self.canvas_drag = Some(CanvasDrag::MoveAnnotation {
                        index,
                        grab_offset: ImagePoint {
                            x: point.x - anchor.x,
                            y: point.y - anchor.y,
                        },
                    });
                    self.status = "正在拖动标注。".to_string();
                } else if matches!(self.tool, Tool::Rectangle | Tool::Arrow) {
                    self.selected_annotation = None;
                    let start = snap_point_to_edges(clamp_point_to_image(point, image_size), image_size);
                    self.canvas_drag = Some(match self.tool {
                        Tool::Rectangle => CanvasDrag::NewRectangle(DragState {
                            start,
                            current: start,
                        }),
                        Tool::Arrow => CanvasDrag::NewArrow(DragState {
                            start,
                            current: start,
                        }),
                        Tool::Text => unreachable!(),
                    });
                }
            }
        }

        if response.dragged() {
            if let Some(point) = response
                .interact_pointer_pos()
                .and_then(|pos| image_pos_from_screen(pos, image_rect, image_size))
            {
                match &mut self.canvas_drag {
                    Some(CanvasDrag::NewRectangle(drag)) => {
                        drag.current =
                            snap_point_to_edges(clamp_point_to_image(point, image_size), image_size);
                    }
                    Some(CanvasDrag::NewArrow(drag)) => {
                        drag.current =
                            snap_point_to_edges(clamp_point_to_image(point, image_size), image_size);
                    }
                    Some(CanvasDrag::MoveAnnotation { index, grab_offset }) => {
                        if let Some(annotation) = self.annotations.get_mut(*index) {
                            let new_anchor = ImagePoint {
                                x: point.x - grab_offset.x,
                                y: point.y - grab_offset.y,
                            };
                            let new_anchor = snap_point_to_edges(
                                clamp_annotation_anchor(annotation, new_anchor, image_size),
                                image_size,
                            );
                            move_annotation_to(annotation, new_anchor, image_size);
                        }
                    }
                    None => {}
                }
            }
        }

        if response.drag_stopped() {
            if let Some(drag) = self.canvas_drag.take() {
                match drag {
                    CanvasDrag::NewRectangle(drag) => {
                        self.annotations.push(Annotation::Rectangle {
                            from: drag.start,
                            to: drag.current,
                            color: self.color,
                            stroke_width: self.stroke_width,
                        });
                        self.selected_annotation = Some(self.annotations.len().saturating_sub(1));
                        self.status = "已插入矩形标注。".to_string();
                    }
                    CanvasDrag::NewArrow(drag) => {
                        self.annotations.push(Annotation::Arrow {
                            from: drag.start,
                            to: drag.current,
                            color: self.color,
                            stroke_width: self.stroke_width,
                        });
                        self.selected_annotation = Some(self.annotations.len().saturating_sub(1));
                        self.status = "已插入箭头标注。".to_string();
                    }
                    CanvasDrag::MoveAnnotation { index, .. } => {
                        self.selected_annotation = Some(index);
                        self.status = "已移动标注。".to_string();
                    }
                }
            }
        } else if response.clicked() {
            if let Some((point, pos)) = response.interact_pointer_pos().and_then(|pos| {
                image_pos_from_screen(pos, image_rect, image_size).map(|point| (point, pos))
            }) {
                if let Some(index) = self.hit_test_annotation(point) {
                    self.selected_annotation = Some(index);
                } else if self.tool == Tool::Text {
                    self.selected_annotation = None;
                    self.text_draft = Some(TextDraft {
                        at: point,
                        screen_pos: pos,
                        text: String::new(),
                        edit_index: None,
                    });
                    self.status = "请输入标注文字，然后点击插入。".to_string();
                } else {
                    self.selected_annotation = None;
                }
            }
        }

        let painter = ui.painter_at(image_rect);
        for (index, annotation) in self.annotations.iter().enumerate() {
            paint_annotation(
                &painter,
                annotation,
                image_rect,
                image_size,
                self.selected_annotation == Some(index),
            );
        }
        if let Some(preview) = self.preview_annotation() {
            paint_annotation(&painter, &preview, image_rect, image_size, false);
        }

        ctx.request_repaint();
    }

    fn draw_text_draft_overlay(&mut self, ctx: &Context) {
        let Some(mut draft) = self.text_draft.clone() else {
            return;
        };

        let area = egui::Area::new(Id::new("text_draft_overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(draft.screen_pos + Vec2::new(12.0, 12.0));

        let mut keep_draft = true;
        let mut insert_text = false;

        area.show(ctx, |ui| {
            Frame::popup(ui.style())
                .fill(Color32::from_rgb(30, 33, 40))
                .stroke(Stroke::new(1.0, Color32::from_gray(70)))
                .rounding(Rounding::same(12.0))
                .show(ui, |ui| {
                ui.set_min_width(280.0);
                ui.label(RichText::new("插入文字").strong().size(18.0));
                ui.add(
                    egui::TextEdit::multiline(&mut draft.text)
                        .desired_width(260.0)
                        .desired_rows(3)
                        .hint_text("输入中文标注，例如：这里需要重点说明"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("插入").fill(Color32::from_rgb(24, 119, 242)))
                        .clicked()
                    {
                        insert_text = true;
                    }
                    if ui.button("取消").clicked() {
                        keep_draft = false;
                    }
                });
            });
        });

        if insert_text {
            if draft.text.trim().is_empty() {
                self.status = "文字内容为空，未插入任何标注。".to_string();
                self.text_draft = None;
            } else {
                if let Some(index) = draft.edit_index {
                    if let Some(Annotation::Text {
                        at,
                        text,
                        color,
                        size,
                    }) = self.annotations.get_mut(index)
                    {
                        *at = clamp_text_anchor(draft.at, &draft.text, *size, image_size_vec(&self.base_image));
                        *text = draft.text;
                        *color = self.color;
                        *size = self.text_size;
                        self.selected_annotation = Some(index);
                        self.status = "已更新文字标注。".to_string();
                    }
                } else {
                    self.annotations.push(Annotation::Text {
                        at: clamp_text_anchor(
                            draft.at,
                            &draft.text,
                            self.text_size,
                            image_size_vec(&self.base_image),
                        ),
                        text: draft.text,
                        color: self.color,
                        size: self.text_size,
                    });
                    self.selected_annotation = Some(self.annotations.len().saturating_sub(1));
                    self.status = "已插入文字标注。".to_string();
                }
                self.text_draft = None;
            }
        } else if keep_draft {
            self.text_draft = Some(draft);
        } else {
            self.text_draft = None;
            self.status = "已取消文字输入。".to_string();
        }
    }

    fn undo_last_annotation(&mut self) {
        if self.annotations.pop().is_some() {
            self.selected_annotation = self.annotations.len().checked_sub(1);
            self.status = "已撤销最后一个标注。".to_string();
            self.output_feedback = None;
        } else {
            self.status = "当前没有可撤销的标注。".to_string();
        }
    }

    fn hit_test_annotation(&self, point: ImagePoint) -> Option<usize> {
        self.annotations
            .iter()
            .enumerate()
            .rev()
            .find(|(_, annotation)| annotation_hit_test(annotation, point))
            .map(|(index, _)| index)
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        let undo = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Z));
        if undo {
            self.undo_last_annotation();
        }

        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if escape && self.text_draft.is_some() {
            self.text_draft = None;
            self.status = "已取消文字输入。".to_string();
        }

        let save = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S));
        if save {
            self.save_annotated_image();
        }

        let copy = ctx.input(|input| {
            input.modifiers.command
                && input.modifiers.shift
                && input.key_pressed(egui::Key::C)
        });
        if copy {
            self.copy_annotated_image();
        }
    }

    fn preview_annotation(&self) -> Option<Annotation> {
        match &self.canvas_drag {
            Some(CanvasDrag::NewRectangle(drag)) => Some(Annotation::Rectangle {
                from: drag.start,
                to: drag.current,
                color: self.color,
                stroke_width: self.stroke_width,
            }),
            Some(CanvasDrag::NewArrow(drag)) => Some(Annotation::Arrow {
                from: drag.start,
                to: drag.current,
                color: self.color,
                stroke_width: self.stroke_width,
            }),
            _ => None,
        }
    }

    fn start_text_edit(&mut self, index: usize) {
        if let Some(Annotation::Text { at, text, .. }) = self.annotations.get(index).cloned() {
            self.text_draft = Some(TextDraft {
                at,
                screen_pos: Pos2::new(280.0, 120.0),
                text,
                edit_index: Some(index),
            });
            self.selected_annotation = Some(index);
            self.status = "正在编辑选中的文字标注。".to_string();
        }
    }
}

impl eframe::App for AnnotationEditorApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.ensure_texture(ctx);
        self.handle_shortcuts(ctx);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            Frame::none()
                .fill(Color32::from_rgb(18, 20, 26))
                .inner_margin(Margin::symmetric(20.0, 16.0))
                .show(ui, |ui| {
                    self.draw_header(ui);
                });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            Frame::none()
                .fill(Color32::from_rgb(18, 20, 26))
                .inner_margin(Margin::symmetric(20.0, 12.0))
                .show(ui, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(RichText::new("状态").strong().color(Color32::from_gray(210)));
                        ui.separator();
                        ui.label(&self.status);
                        if let Some(feedback) = &self.output_feedback {
                            ui.separator();
                            ui.label(RichText::new(feedback).color(Color32::from_gray(175)));
                        }
                    });
                });
        });

        egui::SidePanel::left("inspector")
            .exact_width(260.0)
            .resizable(false)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(Color32::from_rgb(14, 16, 22))
                    .inner_margin(Margin::same(16.0))
                    .show(ui, |ui| {
                        self.draw_side_panel(ui);
                        self.draw_toolbar(ui);
                    });
            });

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(10, 12, 18))
                    .inner_margin(Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                Frame::none()
                    .fill(Color32::from_rgb(22, 24, 31))
                    .stroke(Stroke::new(1.0, Color32::from_gray(52)))
                    .rounding(Rounding::same(16.0))
                    .inner_margin(Margin::same(18.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("画布")
                                    .size(18.0)
                                    .strong()
                                    .color(Color32::from_rgb(240, 242, 245)),
                            );
                            ui.separator();
                            ui.label(
                                RichText::new(format!(
                                    "{} × {}",
                                    self.base_image.resolution.width, self.base_image.resolution.height
                                ))
                                .color(Color32::from_gray(160)),
                            );
                        });
                        ui.add_space(12.0);
                        egui::ScrollArea::both().show(ui, |ui| {
                            self.draw_image_canvas(ui, ctx);
                        });
                    });
            });
        self.draw_text_draft_overlay(ctx);
    }
}

fn configure_visuals(ctx: &Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(14, 16, 22);
    visuals.window_fill = Color32::from_rgb(22, 24, 31);
    visuals.extreme_bg_color = Color32::from_rgb(11, 13, 18);
    visuals.faint_bg_color = Color32::from_rgb(34, 37, 46);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(36, 39, 48);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 52, 62);
    visuals.widgets.active.bg_fill = Color32::from_rgb(24, 119, 242);
    visuals.selection.bg_fill = Color32::from_rgb(24, 119, 242);
    visuals.window_rounding = Rounding::same(14.0);
    ctx.set_visuals(visuals);
}

fn configure_fonts(ctx: &Context, font_path: Option<&PathBuf>) {
    let Some(path) = font_path else {
        return;
    };
    let Ok(bytes) = fs::read(path) else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("annotation-font".to_string(), FontData::from_owned(bytes));
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "annotation-font".to_string());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "annotation-font".to_string());
    ctx.set_fonts(fonts);
}

fn load_font(path: Option<&PathBuf>) -> Result<FontArc, CaptureError> {
    let path = path.ok_or_else(|| CaptureError::Unsupported {
        message: "no usable font found for text annotation export".to_string(),
    })?;
    let bytes = fs::read(path).map_err(|error| CaptureError::Output {
        message: format!("failed to read annotation font {}: {error}", path.display()),
    })?;
    FontArc::try_from_vec(bytes).map_err(|_| CaptureError::Unsupported {
        message: format!(
            "font {} is not a supported TTF/OTF font for annotation export",
            path.display()
        ),
    })
}

fn paint_annotation(
    painter: &egui::Painter,
    annotation: &Annotation,
    image_rect: Rect,
    image_size: Vec2,
    _selected: bool,
) {
    match annotation {
        Annotation::Rectangle {
            from,
            to,
            color,
            stroke_width,
        } => {
            let start = screen_pos(*from, image_rect, image_size);
            let end = screen_pos(*to, image_rect, image_size);
            painter.rect_stroke(
                Rect::from_two_pos(start, end),
                0.0,
                Stroke::new(*stroke_width as f32, *color),
            );
        }
        Annotation::Arrow {
            from,
            to,
            color,
            stroke_width,
        } => {
            draw_arrow_painter(painter, *from, *to, image_rect, image_size, *color, *stroke_width);
        }
        Annotation::Text {
            at,
            text,
            color,
            size,
        } => {
            let top_left = screen_pos(*at, image_rect, image_size);
            let bounds = text_bounds(*at, text, *size);
            let min = screen_pos(
                ImagePoint {
                    x: bounds.min_x as f32,
                    y: bounds.min_y as f32,
                },
                image_rect,
                image_size,
            );
            let max = screen_pos(
                ImagePoint {
                    x: (bounds.min_x + bounds.width) as f32,
                    y: (bounds.min_y + bounds.height) as f32,
                },
                image_rect,
                image_size,
            );
            let rect = Rect::from_min_max(min, max);
            painter.rect_filled(rect, 6.0, text_background(*color));
            painter.text(
                top_left,
                egui::Align2::LEFT_TOP,
                text,
                FontId::new(*size, FontFamily::Proportional),
                *color,
            );
        }
    }
}

fn image_pos_from_screen(pos: Pos2, rect: Rect, image_size: Vec2) -> Option<ImagePoint> {
    if !rect.contains(pos) {
        return None;
    }

    let x = ((pos.x - rect.min.x) / rect.width() * image_size.x).clamp(0.0, image_size.x);
    let y = ((pos.y - rect.min.y) / rect.height() * image_size.y).clamp(0.0, image_size.y);
    Some(ImagePoint { x, y })
}

fn screen_pos(point: ImagePoint, rect: Rect, image_size: Vec2) -> Pos2 {
    Pos2::new(
        rect.min.x + (point.x / image_size.x) * rect.width(),
        rect.min.y + (point.y / image_size.y) * rect.height(),
    )
}

struct Bounds {
    min_x: u32,
    min_y: u32,
    width: u32,
    height: u32,
}

fn normalized_bounds(from: ImagePoint, to: ImagePoint) -> Bounds {
    let min_x = from.x.min(to.x).round().max(0.0) as u32;
    let min_y = from.y.min(to.y).round().max(0.0) as u32;
    let max_x = from.x.max(to.x).round().max(min_x as f32) as u32;
    let max_y = from.y.max(to.y).round().max(min_y as f32) as u32;

    Bounds {
        min_x,
        min_y,
        width: (max_x.saturating_sub(min_x)).max(1),
        height: (max_y.saturating_sub(min_y)).max(1),
    }
}

fn rgba(color: Color32) -> Rgba<u8> {
    Rgba([color.r(), color.g(), color.b(), color.a()])
}

fn text_background(color: Color32) -> Color32 {
    let luminance =
        0.2126 * f32::from(color.r()) + 0.7152 * f32::from(color.g()) + 0.0722 * f32::from(color.b());
    if luminance > 150.0 {
        Color32::from_rgba_premultiplied(18, 22, 28, 128)
    } else {
        Color32::from_rgba_premultiplied(0, 0, 0, 0)
    }
}

fn annotation_anchor(annotation: &Annotation) -> ImagePoint {
    match annotation {
        Annotation::Rectangle { from, to, .. } => ImagePoint {
            x: from.x.min(to.x),
            y: from.y.min(to.y),
        },
        Annotation::Arrow { from, .. } => *from,
        Annotation::Text { at, .. } => *at,
    }
}

fn move_annotation_to(annotation: &mut Annotation, new_anchor: ImagePoint, image_size: Vec2) {
    match annotation {
        Annotation::Rectangle { from, to, .. } => {
            let current_anchor = ImagePoint {
                x: from.x.min(to.x),
                y: from.y.min(to.y),
            };
            let dx = new_anchor.x - current_anchor.x;
            let dy = new_anchor.y - current_anchor.y;
            from.x += dx;
            from.y += dy;
            to.x += dx;
            to.y += dy;
        }
        Annotation::Arrow { from, to, .. } => {
            let dx = new_anchor.x - from.x;
            let dy = new_anchor.y - from.y;
            from.x = new_anchor.x;
            from.y = new_anchor.y;
            to.x = clamp_coord(to.x + dx, image_size.x);
            to.y = clamp_coord(to.y + dy, image_size.y);
        }
        Annotation::Text { at, .. } => {
            *at = new_anchor;
        }
    }
}

fn annotation_hit_test(annotation: &Annotation, point: ImagePoint) -> bool {
    match annotation {
        Annotation::Rectangle { from, to, .. } => {
            let bounds = normalized_bounds(*from, *to);
            point.x >= bounds.min_x as f32 - 6.0
                && point.x <= (bounds.min_x + bounds.width) as f32 + 6.0
                && point.y >= bounds.min_y as f32 - 6.0
                && point.y <= (bounds.min_y + bounds.height) as f32 + 6.0
        }
        Annotation::Text { at, text, size, .. } => {
            let bounds = text_bounds(*at, text, *size);
            point.x >= bounds.min_x as f32
                && point.x <= (bounds.min_x + bounds.width) as f32
                && point.y >= bounds.min_y as f32
                && point.y <= (bounds.min_y + bounds.height) as f32
        }
        Annotation::Arrow { from, to, .. } => distance_to_segment(point, *from, *to) <= 10.0,
    }
}

fn text_bounds(at: ImagePoint, text: &str, size: f32) -> Bounds {
    let lines: Vec<&str> = text.lines().collect();
    let line_count = lines.len().max(1) as f32;
    let max_chars = lines.iter().map(|line| line.chars().count()).max().unwrap_or(1) as f32;
    let width = (max_chars * size * 0.62 + 14.0).round().max(1.0) as u32;
    let height = (line_count * size * 1.35 + 10.0).round().max(1.0) as u32;

    Bounds {
        min_x: at.x.round().max(0.0) as u32,
        min_y: at.y.round().max(0.0) as u32,
        width,
        height,
    }
}

fn image_size_vec(image: &CapturedImage) -> Vec2 {
    Vec2::new(image.resolution.width as f32, image.resolution.height as f32)
}

fn clamp_point_to_image(point: ImagePoint, image_size: Vec2) -> ImagePoint {
    ImagePoint {
        x: clamp_coord(point.x, image_size.x),
        y: clamp_coord(point.y, image_size.y),
    }
}

fn clamp_coord(value: f32, max: f32) -> f32 {
    value.clamp(0.0, max.max(0.0))
}

fn snap_point_to_edges(point: ImagePoint, image_size: Vec2) -> ImagePoint {
    const SNAP: f32 = 12.0;
    let mut point = point;
    if point.x <= SNAP {
        point.x = 0.0;
    }
    if point.y <= SNAP {
        point.y = 0.0;
    }
    if (image_size.x - point.x).abs() <= SNAP {
        point.x = image_size.x;
    }
    if (image_size.y - point.y).abs() <= SNAP {
        point.y = image_size.y;
    }
    point
}

fn clamp_annotation_anchor(annotation: &Annotation, new_anchor: ImagePoint, image_size: Vec2) -> ImagePoint {
    match annotation {
        Annotation::Rectangle { from, to, .. } => {
            let bounds = normalized_bounds(*from, *to);
            ImagePoint {
                x: new_anchor.x.clamp(0.0, (image_size.x - bounds.width as f32).max(0.0)),
                y: new_anchor.y.clamp(0.0, (image_size.y - bounds.height as f32).max(0.0)),
            }
        }
        Annotation::Arrow { from, to, .. } => {
            let dx = to.x - from.x;
            let dy = to.y - from.y;
            let min_x = 0.0f32.min(-dx);
            let max_x = image_size.x.min(image_size.x - dx);
            let min_y = 0.0f32.min(-dy);
            let max_y = image_size.y.min(image_size.y - dy);
            ImagePoint {
                x: new_anchor.x.clamp(min_x, max_x),
                y: new_anchor.y.clamp(min_y, max_y),
            }
        }
        Annotation::Text { text, size, .. } => clamp_text_anchor(new_anchor, text, *size, image_size),
    }
}

fn clamp_text_anchor(at: ImagePoint, text: &str, size: f32, image_size: Vec2) -> ImagePoint {
    let bounds = text_bounds(at, text, size);
    ImagePoint {
        x: at.x.clamp(0.0, (image_size.x - bounds.width as f32).max(0.0)),
        y: at.y.clamp(0.0, (image_size.y - bounds.height as f32).max(0.0)),
    }
}

fn distance_to_segment(point: ImagePoint, start: ImagePoint, end: ImagePoint) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
        return ((point.x - start.x).powi(2) + (point.y - start.y).powi(2)).sqrt();
    }
    let t = (((point.x - start.x) * dx) + ((point.y - start.y) * dy)) / (dx * dx + dy * dy);
    let t = t.clamp(0.0, 1.0);
    let proj_x = start.x + t * dx;
    let proj_y = start.y + t * dy;
    ((point.x - proj_x).powi(2) + (point.y - proj_y).powi(2)).sqrt()
}

fn draw_arrow_mut(
    image: &mut RgbaImage,
    from: ImagePoint,
    to: ImagePoint,
    color: Color32,
    stroke_width: u32,
) {
    let shaft = rgba(color);
    for offset in 0..stroke_width.max(1) {
        let spread = offset as f32 - (stroke_width.max(1) as f32 - 1.0) / 2.0;
        draw_line_segment_mut(
            image,
            (from.x + spread, from.y + spread),
            (to.x + spread, to.y + spread),
            shaft,
        );
    }

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let arrow_len = (12.0 + stroke_width as f32 * 1.5).min(length * 0.6);
    let wing_x = -uy;
    let wing_y = ux;
    let head_left = ImagePoint {
        x: to.x - ux * arrow_len + wing_x * arrow_len * 0.45,
        y: to.y - uy * arrow_len + wing_y * arrow_len * 0.45,
    };
    let head_right = ImagePoint {
        x: to.x - ux * arrow_len - wing_x * arrow_len * 0.45,
        y: to.y - uy * arrow_len - wing_y * arrow_len * 0.45,
    };
    draw_line_segment_mut(image, (to.x, to.y), (head_left.x, head_left.y), shaft);
    draw_line_segment_mut(image, (to.x, to.y), (head_right.x, head_right.y), shaft);
}

fn draw_arrow_painter(
    painter: &egui::Painter,
    from: ImagePoint,
    to: ImagePoint,
    image_rect: Rect,
    image_size: Vec2,
    color: Color32,
    stroke_width: u32,
) {
    let from = screen_pos(from, image_rect, image_size);
    let to = screen_pos(to, image_rect, image_size);
    painter.line_segment([from, to], Stroke::new(stroke_width as f32, color));

    let direction = to - from;
    let length = direction.length().max(1.0);
    let unit = direction / length;
    let wing = Vec2::new(-unit.y, unit.x);
    let arrow_len = (12.0 + stroke_width as f32 * 1.5).min(length * 0.6);
    let left = to - unit * arrow_len + wing * arrow_len * 0.45;
    let right = to - unit * arrow_len - wing * arrow_len * 0.45;
    painter.line_segment([to, left], Stroke::new(stroke_width as f32, color));
    painter.line_segment([to, right], Stroke::new(stroke_width as f32, color));
}

fn color_menu_button(ui: &mut egui::Ui, color: &mut Color32) {
    ui.menu_button(
        RichText::new("      ")
            .background_color(*color)
            .color(*color),
        |ui| {
            ui.label("常用颜色");
            ui.horizontal_wrapped(|ui| {
                for (label, preset) in PRESET_COLORS {
                    let selected = *color == preset;
                    let response = ui.add(
                        egui::Button::new("   ")
                            .fill(preset)
                            .stroke(if selected {
                                Stroke::new(2.0, Color32::WHITE)
                            } else {
                                Stroke::new(1.0, Color32::from_gray(70))
                            })
                            .min_size(Vec2::new(24.0, 24.0)),
                    );
                    if response.clicked() {
                        *color = preset;
                        ui.close_menu();
                    }
                    response.on_hover_text(label);
                }
            });
            ui.separator();
            ui.label("自定义颜色");
            ui.color_edit_button_srgba(color);
        },
    );
}

fn section_card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    Frame::none()
        .fill(Color32::from_rgb(24, 27, 34))
        .stroke(Stroke::new(1.0, Color32::from_gray(48)))
        .rounding(Rounding::same(14.0))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().size(16.0));
            ui.add_space(10.0);
            add_contents(ui);
        });
    ui.add_space(12.0);
}

fn tool_chip(ui: &mut egui::Ui, current: &mut Tool, value: Tool, label: &str) {
    let selected = *current == value;
    let button = egui::Button::new(RichText::new(label).strong())
        .fill(if selected {
            Color32::from_rgb(24, 119, 242)
        } else {
            Color32::from_rgb(40, 44, 54)
        })
        .stroke(Stroke::new(
            1.0,
            if selected {
                Color32::from_rgb(101, 168, 255)
            } else {
                Color32::from_gray(60)
            },
        ))
        .min_size(Vec2::new(104.0, 34.0));
    if ui.add(button).clicked() {
        *current = value;
    }
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, CaptureError> {
    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ColorType::Rgba8.into(),
        )
        .map_err(|error| CaptureError::Encoding {
            message: format!("failed to encode annotated PNG: {error}"),
        })?;
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::{ImagePoint, normalized_bounds};

    #[test]
    fn normalized_bounds_orders_points() {
        let bounds = normalized_bounds(
            ImagePoint { x: 120.0, y: 200.0 },
            ImagePoint { x: 20.0, y: 50.0 },
        );

        assert_eq!(bounds.min_x, 20);
        assert_eq!(bounds.min_y, 50);
        assert_eq!(bounds.width, 100);
        assert_eq!(bounds.height, 150);
    }
}
