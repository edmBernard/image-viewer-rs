// Egui panels: bottom menu, settings, shortcut editor, image list and review panel

use bevy::prelude::*;
use bevy_egui::egui::CollapsingHeader;
use bevy_egui::{egui, EguiContexts};

use crate::{
    get_short_name, key_label, ActivateReviewEvent, AddMode, ChangeSamplerEvent, ChangeTitleStyleEvent, Config,
    FitToScreen, GlobalScale, GridLayout, GridLayoutState, ImageList, ImagePath, MoveImageEvent, MultiCursorEnabled,
    MyAppState, MyImage, NavigateReviewEvent, RecordedPressedKey, RefreshReviewEvent, ReloadImagesEvent,
    RemoveImageEvent, ResetScales, ReviewState, SamplerMode, SaveCropped, SaveSettingsEvent, ScreenColorSpace,
    ScrollBehavior, UiPanelArea, UiState,
};

// MARK: Panel Root
struct PanelRootUi<'a> {
    ui: egui::Ui,
    area: &'a mut UiPanelArea,
}

impl<'a> PanelRootUi<'a> {
    fn new(ctx: &egui::Context, id: &str, area: &'a mut UiPanelArea) -> Self {
        let pass_nr = ctx.cumulative_pass_nr();
        if area.pass_nr != pass_nr {
            area.pass_nr = pass_nr;
            area.rect = None;
        }
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new(id),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(area.rect.unwrap_or_else(|| ctx.viewport_rect())),
        );
        PanelRootUi { ui, area }
    }
}

impl Drop for PanelRootUi<'_> {
    fn drop(&mut self) {
        self.area.rect = Some(self.ui.available_rect_before_wrap());
    }
}

impl std::ops::Deref for PanelRootUi<'_> {
    type Target = egui::Ui;
    fn deref(&self) -> &egui::Ui {
        &self.ui
    }
}

impl std::ops::DerefMut for PanelRootUi<'_> {
    fn deref_mut(&mut self) -> &mut egui::Ui {
        &mut self.ui
    }
}

// MARK: UI Systems
pub fn configure_visuals(mut egui_ctx: EguiContexts) {
    let Ok(ctx) = egui_ctx.ctx_mut() else { return };
    ctx.set_visuals(egui::Visuals { ..Default::default() });
}

pub fn ui_bottom_menu(
    mut contexts: EguiContexts,
    mut layout_state: ResMut<GridLayoutState>,
    mut ui_state: ResMut<UiState>,
    mut global_scale: ResMut<GlobalScale>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut save_cropped_evw: MessageWriter<SaveCropped>,
    mut reset_scales_evw: MessageWriter<ResetScales>,
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
    mut add_mode: ResMut<AddMode>,
    mut review_state: ResMut<ReviewState>,
    mut activate_evw: MessageWriter<ActivateReviewEvent>,
    mut panel_area: ResMut<UiPanelArea>,
    image_list: Res<ImageList>,
) {
    if ui_state.visible {
        let Ok(ctx) = contexts.ctx_mut() else { return };
        let mut root = PanelRootUi::new(ctx, "root_bottom_menu", &mut panel_area);
        egui::Panel::bottom("wrap_app_top_bar").show(&mut root, |ui| {
            // equivalent to horizontal_wrapped but with a small factor on y to avoid the clip of button
            let initial_size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y * 1.2);
            ui.allocate_ui_with_layout(
                initial_size,
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                |ui| {
                    egui::widgets::global_theme_preference_switch(ui);
                    ui.toggle_value(&mut ui_state.settings_panel_visible, "\u{2699}");
                    ui.toggle_value(&mut ui_state.image_list_visible, "\u{2630}");
                    ui.separator();
                    let mut scale = global_scale.0.log2();

                    if ui
                        .add(
                            egui::DragValue::new(&mut scale)
                                .prefix("\u{1F50E} ")
                                .speed(0.1)
                                .range(-10.0..=10.),
                        )
                        .on_hover_text("Zoom")
                        .changed()
                    {
                        global_scale.0 = 2f32.powf(scale);
                    }

                    if ui.button("1:1").on_hover_text("Reset All Zoom").clicked() {
                        reset_scales_evw.write(ResetScales);
                    }

                    if ui.button("Fit").on_hover_text("Fit to Screen").clicked() {
                        fit_to_screen_evw.write(FitToScreen);
                    }

                    // Always 10 buttons so the bar doesn't move; the ones without an image are disabled
                    for i in 0..10 {
                        let mut state = i == layout_state.index;
                        ui.add_enabled_ui(i < image_list.0.len(), |ui| {
                            if ui.toggle_value(&mut state, format!("{}", i + 1)).changed() {
                                layout_state.index = i;
                                move_image_evw.write(MoveImageEvent);
                            }
                        });
                    }

                    ui.separator();
                    for layout in GridLayout::ALL {
                        if ui
                            .selectable_value(&mut layout_state.layout, layout, layout.label())
                            .changed()
                        {
                            move_image_evw.write(MoveImageEvent);
                        }
                    }

                    ui.separator();
                    if ui
                        .button("\u{26F6}")
                        .on_hover_text("Save crops next to original images suffixed with _crop")
                        .clicked()
                    {
                        save_cropped_evw.write(SaveCropped);
                    }

                    ui.separator();
                    ui.toggle_value(&mut add_mode.0, "Add")
                        .on_hover_text("When enabled, dropped images are added instead of replacing");

                    ui.separator();
                    let review_toggled = ui
                        .toggle_value(&mut review_state.enabled, "Review")
                        .on_hover_text("Review mode: navigate through similar images")
                        .changed();
                    if review_toggled && review_state.enabled && review_state.radixes.is_empty() {
                        activate_evw.write(ActivateReviewEvent);
                    }
                },
            );
        });
    }
}

pub fn ui_settings_menu(
    mut contexts: EguiContexts,
    mut config: ResMut<Config>,
    // Index in ConfigShortcut::entries_mut of the shortcut being recorded
    mut editing_shortcut: Local<Option<usize>>,
    mut recorded_key: ResMut<RecordedPressedKey>,
    ui_state: Res<UiState>,
    mut cursor_state: ResMut<MultiCursorEnabled>,
    mut change_title_style_evw: MessageWriter<ChangeTitleStyleEvent>,
    mut save_settings_evw: MessageWriter<SaveSettingsEvent>,
    mut change_sampler_evw: MessageWriter<ChangeSamplerEvent>,
    mut reload_images_evw: MessageWriter<ReloadImagesEvent>,
    mut next_state: ResMut<NextState<MyAppState>>,
    mut panel_area: ResMut<UiPanelArea>,
) {
    if ui_state.settings_panel_visible {
        let Ok(ctx) = contexts.ctx_mut() else { return };
        let mut root = PanelRootUi::new(ctx, "root_settings", &mut panel_area);
        egui::Panel::right("Settings").resizable(false).show(&mut root, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Settings");
                ui.hyperlink_to(
                    format!("{} Source Code", egui::special_emojis::GITHUB),
                    "https://github.com/edmBernard/image-viewer-rs",
                );
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Behavior on Scroll:");
                    egui::ComboBox::from_label("")
                        .selected_text(format!("{:?}", config.misc.scroll_behavior))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut config.misc.scroll_behavior, ScrollBehavior::None, "Disabled");
                            ui.selectable_value(&mut config.misc.scroll_behavior, ScrollBehavior::Move, "Move");
                            ui.selectable_value(&mut config.misc.scroll_behavior, ScrollBehavior::Zoom, "Zoom");
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("Interpolation:");
                    egui::ComboBox::from_label(" ")
                        .selected_text(format!("{:?}", config.misc.sampler_mode))
                        .show_ui(ui, |ui| {
                            let r1 = ui
                                .selectable_value(&mut config.misc.sampler_mode, SamplerMode::Nearest, "Nearest")
                                .changed();
                            let r2 = ui
                                .selectable_value(&mut config.misc.sampler_mode, SamplerMode::Bilinear, "Bilinear")
                                .changed();
                            if r1 || r2 {
                                change_sampler_evw.write(ChangeSamplerEvent);
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("Screen Color Space:")
                        .on_hover_text("Color space of the physical screen; images are converted to it");
                    egui::ComboBox::from_id_salt("screen_color_space")
                        .selected_text(config.color.screen_color_space.label())
                        .show_ui(ui, |ui| {
                            let mut changed = false;
                            for space in ScreenColorSpace::ALL {
                                changed |= ui
                                    .selectable_value(&mut config.color.screen_color_space, space, space.label())
                                    .changed();
                            }
                            if changed {
                                reload_images_evw.write(ReloadImagesEvent);
                            }
                        });
                });

                ui.checkbox(&mut cursor_state.0, "Enable Multi Cursor");

                ui.horizontal(|ui| {
                    ui.label("Grid Width:");
                    ui.add(egui::DragValue::new(&mut config.misc.grid_width));
                });

                CollapsingHeader::new("Style").default_open(true).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Font Size:");
                        if ui
                            .add(egui::Slider::new(&mut config.text.font_size, 8.0..=70.0))
                            .changed()
                        {
                            change_title_style_evw.write(ChangeTitleStyleEvent);
                        }
                    });
                    let mut color_vec = config.text.font_color.to_linear().to_f32_array();
                    ui.horizontal(|ui| {
                        ui.label("Font Color:");
                        if ui.color_edit_button_rgba_unmultiplied(&mut color_vec).changed() {
                            change_title_style_evw.write(ChangeTitleStyleEvent);
                        }
                        ui.label(format!(
                            "rgba: ({:.2}, {:.2}, {:.2}, {:.2})",
                            color_vec[0], color_vec[1], color_vec[2], color_vec[3],
                        ));
                    });
                    config.text.font_color = Color::LinearRgba(LinearRgba::from_f32_array(color_vec));
                });

                CollapsingHeader::new("Short Cut").default_open(true).show(ui, |ui| {
                    // Clicking a shortcut switches to EditShortCut state, where record_pressed_key
                    // catches the next key; it's applied here once back in Working state
                    for (index, (label, key)) in config.shortcut.entries_mut().into_iter().enumerate() {
                        if *editing_shortcut == Some(index) {
                            if let Some(recorded) = recorded_key.0.take() {
                                *key = recorded;
                                *editing_shortcut = None;
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.label(label);
                            let mut editing = *editing_shortcut == Some(index);
                            if ui.toggle_value(&mut editing, key_label(key)).changed() && editing {
                                *editing_shortcut = Some(index);
                                next_state.set(MyAppState::EditShortCut);
                            }
                        });
                    }
                });

                if ui.button("Save Settings").clicked() {
                    save_settings_evw.write(SaveSettingsEvent);
                };
            });
        });
    }
}

pub fn ui_edit_short_cut(mut contexts: EguiContexts, mut panel_area: ResMut<UiPanelArea>) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut root = PanelRootUi::new(ctx, "root_edit_short_cut", &mut panel_area);
    egui::CentralPanel::default().show(&mut root, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
            ui.heading("Press short cut key");
        })
    });
}

pub fn ui_image_list_panel(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut image_list: ResMut<ImageList>,
    image_path_query: Query<&ImagePath, With<MyImage>>,
    mut remove_image_evw: MessageWriter<RemoveImageEvent>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut review_state: ResMut<ReviewState>,
    mut panel_area: ResMut<UiPanelArea>,
) {
    if !ui_state.image_list_visible {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let mut root = PanelRootUi::new(ctx, "root_image_list", &mut panel_area);
    egui::Panel::left("Image List")
        .resizable(true)
        .default_size(180.)
        .show(&mut root, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Images");
            });
            ui.separator();

            let mut to_remove: Option<Entity> = None;
            let mut from_slot: Option<usize> = None;
            let mut to_slot: Option<usize> = None;

            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for (slot, &image_entity) in image_list.0.iter().enumerate() {
                    let name = 'name: {
                        let Ok(path) = image_path_query.get(image_entity) else {
                            break 'name "?";
                        };
                        get_short_name(&path.0).unwrap_or("?")
                    };

                    let row_id = egui::Id::new("image_list_row").with(slot);

                    // Row layout: [drag-source label] [remove button]
                    // The remove button is outside the drag source so it receives clicks properly.
                    ui.horizontal(|ui| {
                        let response = ui
                            .dnd_drag_source(row_id, slot, |ui| {
                                ui.label(format!("{}. {}", slot + 1, name));
                            })
                            .response;

                        if ui.button("\u{2716}").on_hover_text("Remove image").clicked() {
                            to_remove = Some(image_entity);
                        }

                        // Show a drop indicator line and determine insertion point
                        if let (Some(pointer), Some(hovered_payload)) = (
                            ui.input(|i| i.pointer.interact_pos()),
                            response.dnd_hover_payload::<usize>(),
                        ) {
                            let rect = response.rect;
                            let stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

                            let insert_slot = if *hovered_payload == slot {
                                ui.painter().hline(rect.x_range(), rect.center().y, stroke);
                                slot
                            } else if pointer.y < rect.center().y {
                                ui.painter().hline(rect.x_range(), rect.top(), stroke);
                                slot
                            } else {
                                ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
                                slot + 1
                            };

                            if let Some(dragged_payload) = response.dnd_release_payload::<usize>() {
                                from_slot = Some(*dragged_payload);
                                to_slot = Some(insert_slot);
                            }
                        }
                    });
                }
            });

            // Handle reorder (adjust index when moving within the same list)
            if let (Some(src), Some(mut dst)) = (from_slot, to_slot) {
                dst -= (src < dst) as usize;
                if src != dst {
                    move_item(&mut image_list.0, src, dst);
                    // Review patterns are per cell, they follow their image
                    move_item(&mut review_state.cell_patterns, src, dst);
                    move_item(&mut review_state.editable_patterns, src, dst);
                    move_image_evw.write(MoveImageEvent);
                }
            }

            // Handle removal
            if let Some(entity_to_remove) = to_remove {
                remove_image_evw.write(RemoveImageEvent(entity_to_remove));
            }
        });
}

fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to >= items.len() {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

// MARK: Review Panel
pub fn ui_review_panel(
    mut contexts: EguiContexts,
    mut review_state: ResMut<ReviewState>,
    mut navigate_evw: MessageWriter<NavigateReviewEvent>,
    mut refresh_evw: MessageWriter<RefreshReviewEvent>,
    mut activate_evw: MessageWriter<ActivateReviewEvent>,
    ui_state: Res<UiState>,
    mut panel_area: ResMut<UiPanelArea>,
) {
    if !ui_state.visible || !review_state.enabled {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut root = PanelRootUi::new(ctx, "root_review", &mut panel_area);
    egui::Panel::bottom("review_bar").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            if let Some(error) = &review_state.error {
                ui.colored_label(egui::Color32::from_rgb(255, 150, 100), error.as_str());
            } else {
                if ui.button("\u{25C0}").clicked() {
                    navigate_evw.write(NavigateReviewEvent(-1));
                }
                if ui.button("\u{25B6}").clicked() {
                    navigate_evw.write(NavigateReviewEvent(1));
                }

                ui.toggle_value(&mut review_state.drag_switch, "\u{1F4E5}")
                    .on_hover_text("Drag-to-switch: drop an image to jump to its set");

                let total = review_state.radixes.len();
                let current = review_state.current_index;
                let radix_name = review_state.radixes.get(current).cloned().unwrap_or_default();
                ui.label(format!("{}/{}: {}", current + 1, total, radix_name));

                ui.separator();

                for pattern in review_state.editable_patterns.iter_mut() {
                    ui.add(egui::TextEdit::singleline(pattern).desired_width(200.0));
                    ui.separator();
                }

                if ui
                    .button("\u{1F4E4}")
                    .on_hover_text("Reload directory with current regexes")
                    .clicked()
                {
                    refresh_evw.write(RefreshReviewEvent);
                }
            }

            if ui
                .button("\u{21BB}")
                .on_hover_text("Recompute patterns from open images")
                .clicked()
            {
                activate_evw.write(ActivateReviewEvent);
            }
        });
    });
}
