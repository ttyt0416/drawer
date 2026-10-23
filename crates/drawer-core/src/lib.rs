pub mod actions;
pub mod aids;
pub mod animation;
pub mod assets;
pub mod brush_cursor;
pub mod canvas_size;
pub mod controller;
pub mod display_view;
pub mod document;
pub mod files;
pub mod help;
mod history;
pub mod image_import;
pub mod information;
pub mod input_replay;
pub mod interpolation;
pub mod mouse_pressure;
pub mod movie;
pub mod navigator;
pub mod navigator_preferences;
pub mod numeric;
pub mod panels;
pub mod paste;
pub mod png_export;
pub mod recent;
pub mod references;
pub mod render;
pub mod scene;
pub mod selection;
pub mod sizing;
pub mod storage;
#[cfg(test)]
mod test_support;
pub mod thumbnail;
pub mod tool_input;
pub mod ui_behavior;
pub mod ui_scale;
pub mod view;
pub mod view_layout;
pub mod workspaces;

pub mod layout;

pub mod tool_properties;

pub mod preferences;

pub mod acquisition;
pub mod painting;
pub mod pen_controls;
pub mod pointer_input;
pub mod precision;
pub mod pressure;
pub mod tablet_mapping;
pub mod touch;

pub mod basic_brush;

pub mod stroke_ends;

pub mod brush_presets;

pub mod brush_material;

pub mod paper;
pub mod texture;

pub mod psd;

pub mod brush_slots;

pub mod extended_brushes;

pub mod tip_profile;

pub mod stroke_filter;

pub mod color_mix;
pub mod tip_sequence;

pub mod brush_dynamics;

pub mod brush_pattern;

pub mod brush_variation;

pub mod clone_stamp;

pub mod material_library;

pub mod abr;

pub mod brush_composite;

pub mod eyedropper;

pub mod user_tools;

pub mod tool_modes;

pub mod brush_behavior;
pub mod brush_color;
pub mod fibres;
pub mod lab_mix;
pub mod tip_texture;
pub mod tool_bundle;

pub mod line_regions;
pub mod line_tools;

pub mod line_tools_verify;

pub mod tonal;
pub mod tool_locks;

pub mod tonal_verify;

pub mod color_selector;

pub mod color_tables;

pub mod related_colors;

pub mod color_modes;

pub mod screen_sample;

pub mod soft_proof;

pub mod selection_tools;

pub mod selection_paint;

pub mod selection_transfer;

pub mod layer_pick;

pub mod bucket;

pub mod selection_edit;
pub mod selection_filter;
pub mod selection_geometry;

pub mod rulers;

pub mod color_range;

pub mod selection_border;

pub mod fill_reference;

pub mod fill_reference_verify;

pub mod enclosure;
pub mod gradient;
mod gradient_regions;
pub mod gradient_verify;

pub mod gradient_ramp;

pub mod gradient_ramp_verify;

pub mod shapes;

pub mod shapes_verify;

pub mod alignment;
pub mod alignment_verify;

pub mod snapping;

pub mod snapping_verify;

pub mod layer_selection;
pub mod vector_selection;

pub mod layer_selection_verify;

pub mod transparency_verify;

pub mod selection_shortcuts;

pub mod selection_shortcuts_verify;

pub mod magnetic_lasso;

pub mod magnetic_lasso_verify;

pub mod animation_output;
pub mod extended_fill;

pub mod extended_fill_verify;

pub mod auto_shape;

pub mod auto_shape_verify;

pub mod special_rulers;

pub mod ruler_scope;

pub mod special_rulers_verify;

pub mod alignment_detail;

pub mod object_snap;

pub mod alignment_detail_verify;

pub mod layers;

pub mod layers_verify;

pub mod layer_properties;

pub mod layer_properties_verify;

pub mod clipping;

pub mod clipping_verify;

pub mod layer_blend;

pub mod layer_blend_verify;

pub mod layer_effects;

mod individual_merge;
pub mod individual_merge_verify;
pub mod layer_merge;

pub mod layer_merge_verify;

pub mod layer_folders;

pub mod layer_folders_verify;

pub mod adjustments;

pub mod adjustments_verify;

pub mod appearance;
pub mod appearance_verify;
pub mod layer_defaults;
pub mod layer_defaults_verify;
pub mod layer_roles;
pub mod layer_roles_verify;
pub mod layer_search;
pub mod search_move;
pub mod search_verify;

pub mod layer_catalog;

mod catalog_guard;

pub mod search_edit;

pub mod extended_blend_verify;
pub mod file_objects;
pub mod file_objects_verify;
pub mod material_layer;
pub mod material_layer_verify;
pub mod raster_conversion;
pub mod raster_conversion_verify;
pub mod render_order;

pub mod layer_comps;
pub mod layer_comps_verify;

pub mod layer_masks;
pub mod layer_masks_verify;

pub mod transfer_down;
pub mod transfer_down_verify;

pub mod layer_groups;

pub mod group_edits;

pub mod layer_move;

pub mod layer_groups_verify;

pub mod layer_move_verify;

pub mod expression_color;
pub mod expression_color_verify;

pub mod paper_background;
pub mod paper_background_verify;

pub mod color_precision;
pub mod color_precision_verify;
pub mod color_profile;

pub mod color_storage;

#[cfg(test)]
mod color_edit_verify;
#[cfg(test)]
mod color_storage_verify;

pub mod float_render;
#[cfg(test)]
mod float_render_verify;

pub mod layer_maintenance;
pub mod layer_maintenance_verify;

pub mod file_sources;

pub mod file_sources_verify;

pub mod material_mapping;

pub mod material_mapping_verify;

pub mod material_transform;

pub mod material_transform_verify;

pub mod vector_creation_verify;

pub mod vector_pressure;

pub mod vector_edit;

pub mod svg_export;
pub mod svg_import;
pub mod svg_style;
pub mod svg_verify;
pub mod vector_edit_verify;
pub mod vector_erase;
pub mod vector_erase_verify;
pub mod vector_nodes;
pub mod vector_nodes_verify;
pub mod vector_refine;
pub mod vector_refine_verify;
pub mod vector_reset_verify;
pub mod vector_tip;
pub mod vector_tip_verify;

pub mod affine_transform;
pub mod affine_transform_verify;
pub mod batch_filters;
pub mod batch_filters_verify;
pub mod detail_filter;
pub mod detail_filter_verify;
pub mod gaussian;
pub mod gaussian_verify;
pub mod local_color;
pub mod local_color_verify;
pub mod pixel_effects;
pub mod pixel_effects_verify;
pub mod spatial_blur;
pub mod spatial_blur_verify;
pub mod stylized_effects;
pub mod stylized_effects_verify;
pub mod surface_effects;
pub mod surface_effects_verify;
mod surface_kernels;
pub mod tone_mapping;
pub mod tone_mapping_verify;
pub mod vector_cleanup;
pub mod vector_cleanup_verify;
pub mod vector_clipboard;
pub mod vector_scope;
pub mod vector_scope_verify;
pub mod warp_filters;
pub mod warp_filters_verify;

pub mod deform;
pub mod deform_verify;

pub mod mesh_deform;
pub mod mesh_deform_verify;

pub mod color_corrections;
pub mod color_corrections_verify;

pub mod color_deepen;
pub mod color_deepen_verify;

pub mod local_inference;
#[cfg(test)]
mod local_inference_verify;

pub mod metal_material;
pub mod metal_material_verify;

pub mod transform_handles;
pub mod transform_handles_verify;

pub mod mesh_edit;
pub mod transform_policy;

pub mod transform_policy_verify;

pub mod extended_filters;
pub mod extended_filters_verify;

pub mod filter_options;

pub mod filter_options_verify;

pub mod cleanup_filters;

pub mod cleanup_filters_verify;

pub mod quick_filters;

pub mod quick_filters_verify;

pub mod text_layout;

pub mod text_layout_verify;
