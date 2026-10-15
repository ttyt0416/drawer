//! Seven persistent brush slots. Legacy four-slot files retain their original entries.
use serde::{Deserialize, Deserializer, de::Error};
pub const COUNT: usize = 7;
fn with_defaults<'de, D, T>(d: D, mut values: [T; COUNT]) -> Result<[T; COUNT], D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Copy,
{
    let old = Vec::<T>::deserialize(d)?;
    if old.len() != 4 && old.len() != COUNT {
        return Err(D::Error::custom(
            "Expected four legacy or seven brush slots",
        ));
    }
    values[..old.len()].copy_from_slice(&old);
    Ok(values)
}
pub fn read<'de, D, T>(d: D) -> Result<[T; COUNT], D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Copy + Default,
{
    with_defaults(d, [T::default(); COUNT])
}
fn floats(index: usize) -> [f64; COUNT] {
    crate::assets::BrushTip::ALL.map(|t| crate::basic_brush::values(t)[index])
}
pub fn opacity() -> [f64; COUNT] {
    floats(4)
}
pub fn hardness() -> [f64; COUNT] {
    floats(5)
}
pub fn spacing() -> [f64; COUNT] {
    floats(6)
}
pub fn read_opacity<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; COUNT], D::Error> {
    with_defaults(d, opacity())
}
pub fn read_hardness<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; COUNT], D::Error> {
    with_defaults(d, hardness())
}
pub fn read_spacing<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; COUNT], D::Error> {
    with_defaults(d, spacing())
}
pub fn materials() -> [crate::brush_material::Settings; COUNT] {
    crate::assets::BrushTip::ALL.map(crate::basic_brush::material)
}
pub fn read_materials<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<[crate::brush_material::Settings; COUNT], D::Error> {
    with_defaults(d, materials())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        tool_properties::{Property, Tool},
        workspaces::Snapshot,
    };
    #[test]
    fn four_slot_workspace_preserves_values_and_adds_independent_new_defaults() {
        let mut original = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        original.panels.brush_opacity[1] = 0.37;
        original.input.smoothing[0] = 0.6;
        original.input.ends[2].start = 7.0;
        original.input.textures[1].source = 2;
        original.pressure.tools[3].opacity = true;
        original.cursors.tools[2].minimum_pixels = 3.0;
        original
            .properties
            .as_mut()
            .unwrap()
            .tools
            .get_mut(&Tool::Pen)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap()
            .value = 31.0;
        let mut old = serde_json::to_value(&original).unwrap();
        for (parent, fields) in [
            (
                "panels",
                vec!["brush_opacity", "brush_hardness", "brush_spacing"],
            ),
            (
                "input",
                vec![
                    "smoothing",
                    "ends",
                    "materials",
                    "textures",
                    "mouse_pressure",
                ],
            ),
            ("pressure", vec!["tools"]),
            ("cursors", vec!["tools"]),
        ] {
            for field in fields {
                old[parent][field].as_array_mut().unwrap().truncate(4);
            }
        }
        for name in ["Marker", "Smudge", "PixelPen"] {
            old["properties"]["tools"]
                .as_object_mut()
                .unwrap()
                .remove(name);
        }
        let upgraded: Snapshot = serde_json::from_value(old.clone()).unwrap();
        assert!(upgraded.valid());
        assert_eq!(upgraded, original);
        old["input"]["smoothing"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(0.0));
        assert!(serde_json::from_value::<Snapshot>(old).is_err());
    }
    #[test]
    fn old_property_v1_and_virtual_records_upgrade_without_replacing_existing_tools() {
        let settings = crate::tool_properties::Settings::default();
        let mut old = serde_json::to_value(&settings).unwrap();
        old["version"] = serde_json::json!(1);
        for name in ["Marker", "Smudge", "PixelPen"] {
            old["tools"].as_object_mut().unwrap().remove(name);
        }
        for properties in old["tools"].as_object_mut().unwrap().values_mut() {
            properties
                .as_object_mut()
                .unwrap()
                .retain(|k, _| ["Size", "Aspect", "Angle", "Stabilization"].contains(&k.as_str()));
        }
        let mut upgraded: crate::tool_properties::Settings = serde_json::from_value(old).unwrap();
        assert!(upgraded.valid());
        upgraded.upgrade(opacity(), hardness(), spacing());
        assert!(upgraded.valid());
        assert_eq!(upgraded.tools[&Tool::PixelPen][&Property::Size].value, 1.0);
        let mut s = serde_json::to_value(crate::input_replay::Sequence::default()).unwrap();
        for field in [
            "opacity",
            "hardness",
            "spacing",
            "smoothing",
            "ends",
            "materials",
            "textures",
            "mouse_pressure",
        ] {
            s[field].as_array_mut().unwrap().truncate(4);
        }
        let upgraded: crate::input_replay::Sequence = serde_json::from_value(s).unwrap();
        assert!(upgraded.validate().is_ok());
        assert_eq!(
            upgraded.materials[5],
            crate::basic_brush::material(crate::assets::BrushTip::Smudge)
        );
        assert_eq!(upgraded.opacity[4], 0.65);
        assert_eq!(upgraded.spacing[6], 0.25);
    }
}
