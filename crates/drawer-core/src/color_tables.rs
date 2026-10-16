use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Swatch {
    pub id: u64,
    pub name: String,
    pub color: [u8; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub size: u8,
    pub columns: Option<u8>,
    pub scroll: bool,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            size: 24,
            columns: None,
            scroll: true,
        }
    }
}
impl Layout {
    pub fn valid(self) -> bool {
        (12..=64).contains(&self.size) && self.columns.is_none_or(|n| (1..=32).contains(&n))
    }
    pub fn columns_for(self, width: f32) -> usize {
        self.columns.map_or_else(
            || ((width.max(1.0) / (self.size as f32 + 2.0)).floor() as usize).clamp(1, 32),
            usize::from,
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    #[serde(default)]
    pub layout: Layout,
    pub id: u64,
    pub name: String,
    pub colors: Vec<Swatch>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub next_id: u64,
    pub tables: Vec<Table>,
    pub selected: Option<u64>,
    pub auto_sample: bool,
    pub history_size: u8,
    pub history_columns: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            next_id: 2,
            tables: vec![Table {
                layout: Default::default(),
                id: 1,
                name: "Local colors".into(),
                colors: vec![],
            }],
            selected: Some(1),
            auto_sample: false,
            history_size: 24,
            history_columns: 8,
        }
    }
}
fn name(s: &str) -> bool {
    !s.trim().is_empty()
        && s == s.trim()
        && s.chars().count() <= 64
        && !s.chars().any(char::is_control)
}
impl Settings {
    pub fn valid(&self) -> bool {
        let mut ids = BTreeSet::new();
        let mut count = 0;
        self.next_id > 0
            && self.tables.len() <= 16
            && self.tables.iter().all(|t| {
                count += t.colors.len();
                ids.insert(t.id)
                    && t.id > 0
                    && t.id < self.next_id
                    && t.layout.valid()
                    && name(&t.name)
                    && t.colors.len() <= 256
                    && t.colors.iter().all(|c| {
                        ids.insert(c.id) && c.id > 0 && c.id < self.next_id && name(&c.name)
                    })
            })
            && count <= 2048
            && self
                .selected
                .is_none_or(|id| self.tables.iter().any(|t| t.id == id))
            && (12..=64).contains(&self.history_size)
            && (1..=32).contains(&self.history_columns)
    }
    fn edit<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if !self.valid() {
            return Err("Invalid color tables".into());
        }
        let mut next = self.clone();
        let v = f(&mut next)?;
        if !next.valid() {
            return Err("Invalid name/layout or color table capacity".into());
        }
        *self = next;
        Ok(v)
    }
    fn id(&mut self) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or("Color ID limit")?;
        Ok(id)
    }
    pub fn create(&mut self, name: &str) -> Result<u64, String> {
        self.edit(|s| {
            let id = s.id()?;
            s.tables.push(Table {
                layout: Default::default(),
                id,
                name: name.trim().into(),
                colors: vec![],
            });
            s.selected = Some(id);
            Ok(id)
        })
    }
    pub fn remove(&mut self, id: u64) -> Result<(), String> {
        self.edit(|s| {
            let i = s
                .tables
                .iter()
                .position(|t| t.id == id)
                .ok_or("Color table missing")?;
            s.tables.remove(i);
            if s.selected == Some(id) {
                s.selected = s.tables.first().map(|t| t.id)
            }
            Ok(())
        })
    }
    /// A repeated RGBA keeps its stable ID and moves to the end.
    pub fn add(&mut self, color: [u8; 4]) -> Result<u64, String> {
        self.edit(|s| {
            let id = s.selected.ok_or("Select a local color table")?;
            let i = s
                .tables
                .iter()
                .position(|t| t.id == id)
                .ok_or("Color table missing")?;
            let swatch = if let Some(n) = s.tables[i].colors.iter().position(|c| c.color == color) {
                s.tables[i].colors.remove(n)
            } else {
                Swatch {
                    id: s.id()?,
                    name: crate::color_selector::hex(color),
                    color,
                }
            };
            let result = swatch.id;
            s.tables[i].colors.push(swatch);
            Ok(result)
        })
    }
    pub fn add_to(&mut self, table: Option<u64>, color: [u8; 4]) -> Result<u64, String> {
        let mut next = self.clone();
        let selected = next.selected;
        next.selected = table;
        let id = next.add(color)?;
        next.selected = selected;
        if !next.valid() {
            return Err("The selected color table no longer exists".into());
        }
        *self = next;
        Ok(id)
    }

    pub fn edit_color(&mut self, id: u64, name: &str, color: [u8; 4]) -> Result<(), String> {
        self.edit(|s| {
            let c = s
                .tables
                .iter_mut()
                .flat_map(|t| &mut t.colors)
                .find(|c| c.id == id)
                .ok_or("Swatch missing")?;
            c.name = name.trim().into();
            c.color = color;
            Ok(())
        })
    }
    pub fn drop_before(&mut self, source: u64, target: u64, copy: bool) -> Result<u64, String> {
        self.edit(|s| {
            let source_table = s
                .tables
                .iter()
                .position(|t| t.colors.iter().any(|c| c.id == source))
                .ok_or("Source swatch missing")?;
            if !s
                .tables
                .iter()
                .any(|t| t.colors.iter().any(|c| c.id == target))
            {
                return Err("Drop target missing".into());
            }
            if source == target && !copy {
                return Ok(source);
            }
            let source_index = s.tables[source_table]
                .colors
                .iter()
                .position(|c| c.id == source)
                .unwrap();
            let mut entry = if copy {
                s.tables[source_table].colors[source_index].clone()
            } else {
                s.tables[source_table].colors.remove(source_index)
            };
            if copy {
                entry.id = s.id()?;
                entry.name = format!("{} copy", entry.name.chars().take(59).collect::<String>());
            }
            let result = entry.id;
            let t = s
                .tables
                .iter_mut()
                .find(|t| t.colors.iter().any(|c| c.id == target))
                .ok_or("Drop target missing")?;
            let index = t.colors.iter().position(|c| c.id == target).unwrap();
            t.colors.insert(index, entry);
            Ok(result)
        })
    }
    pub fn delete_color(&mut self, id: u64) -> Result<(), String> {
        self.edit(|s| {
            let table = s
                .tables
                .iter_mut()
                .find(|t| t.colors.iter().any(|c| c.id == id))
                .ok_or("Swatch missing")?;
            table.colors.retain(|c| c.id != id);
            Ok(())
        })
    }
    pub fn move_color(&mut self, id: u64, delta: isize) -> Result<(), String> {
        self.edit(|s| {
            let t = s
                .tables
                .iter_mut()
                .find(|t| t.colors.iter().any(|c| c.id == id))
                .ok_or("Swatch missing")?;
            let n = t.colors.iter().position(|c| c.id == id).unwrap();
            let to = n
                .checked_add_signed(delta)
                .filter(|n| *n < t.colors.len())
                .ok_or("Swatch order edge")?;
            t.colors.swap(n, to);
            Ok(())
        })
    }
}
#[derive(Default)]
pub struct History {
    undo: Vec<Settings>,
    redo: Vec<Settings>,
}
impl History {
    pub fn record(&mut self, before: Settings, after: &Settings) {
        if before != *after {
            self.redo.clear();
            self.undo.push(before);
            if self.undo.len() > 16 {
                self.undo.remove(0);
            }
        }
    }
    pub fn undo(&mut self, s: &mut Settings) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(s, before));
        true
    }
    pub fn redo(&mut self, s: &mut Settings) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(s, after));
        true
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}
#[derive(Default)]
pub struct Pixels {
    undo: Vec<Vec<[u8; 4]>>,
    redo: Vec<Vec<[u8; 4]>>,
}
impl Pixels {
    pub fn record(&mut self, before: Vec<[u8; 4]>, after: &[[u8; 4]]) {
        if before != after
            && before.len() == crate::workspaces::PAD_SIZE[0] * crate::workspaces::PAD_SIZE[1]
            && after.len() == before.len()
        {
            self.redo.clear();
            self.undo.push(before);
            if self.undo.len() > 32 {
                self.undo.remove(0);
            }
        }
    }
    pub fn undo(&mut self, p: &mut Vec<[u8; 4]>) -> bool {
        let Some(v) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(p, v));
        true
    }
    pub fn redo(&mut self, p: &mut Vec<[u8; 4]>) -> bool {
        let Some(v) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(p, v));
        true
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}
pub fn verify() -> Result<(), String> {
    let mut s = Settings::default();
    let before = s.clone();
    let a = s.add([20, 80, 160, 0])?;
    s.add([20, 80, 160, 128])?;
    if s.add([20, 80, 160, 0])? != a || s.tables[0].colors.len() != 2 {
        return Err("Swatch RGBA deduplication failed".into());
    }
    s.move_color(a, -1)?;
    let after = s.clone();
    let mut h = History::default();
    h.record(before.clone(), &after);
    if !h.undo(&mut s) || s != before || !h.redo(&mut s) || s != after {
        return Err("Color table undo/redo failed".into());
    }
    let old = s.selected;
    s.remove(old.unwrap())?;
    let remaining = s.clone();
    if s.add_to(old, [1; 4]).is_ok() || s != remaining {
        return Err("Deleted sample table was recreated or redirected".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tables_duplicates_alpha_order_and_own_history() {
        verify().unwrap();
        let mut s = Settings::default();
        let before = s.clone();
        let a = s.add([1, 2, 3, 0]).unwrap();
        let b = s.add([1, 2, 3, 128]).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.add([1, 2, 3, 0]).unwrap(), a);
        assert_eq!(s.tables[0].colors[1].id, a);
        s.move_color(a, -1).unwrap();
        let mut history = History::default();
        history.record(before.clone(), &s);
        let after = s.clone();
        assert!(history.undo(&mut s));
        assert_eq!(s, before);
        assert!(history.redo(&mut s));
        assert_eq!(s, after);
        s.remove(1).unwrap();
        assert!(s.add([0; 4]).is_err());
        let id = s.create("New local table").unwrap();
        assert_eq!(s.selected, Some(id));
    }
    #[test]
    fn table_capacity_is_atomic_and_pixel_undo_is_separate() {
        let mut s = Settings::default();
        for n in 0..256 {
            s.add([n as u8, 0, 0, 255]).unwrap();
        }
        let before = s.clone();
        assert!(s.add([0, 1, 0, 255]).is_err());
        assert_eq!(s, before);
        let mut p = vec![[0; 4]; crate::workspaces::PAD_SIZE[0] * crate::workspaces::PAD_SIZE[1]];
        let before = p.clone();
        p[10] = [20, 30, 40, 128];
        let after = p.clone();
        let mut h = Pixels::default();
        h.record(before.clone(), &p);
        assert!(h.undo(&mut p));
        assert_eq!(p, before);
        assert!(h.redo(&mut p));
        assert_eq!(p, after);
    }
}

#[cfg(test)]
mod editable_tests {
    use super::*;
    #[test]
    fn swatch_move_copy_edit_and_layout_restart() {
        let mut s = Settings::default();
        let a = s.add([1, 2, 3, 0]).unwrap();
        let b = s.add([1, 2, 3, 128]).unwrap();
        let c = s.add([4, 5, 6, 255]).unwrap();
        s.drop_before(c, a, false).unwrap();
        assert_eq!(
            s.tables[0].colors.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![c, a, b]
        );
        let copy = s.drop_before(a, b, true).unwrap();
        assert_ne!(copy, a);
        s.edit_color(copy, "Named color", [1, 2, 3, 0]).unwrap();
        assert_eq!(
            s.tables[0]
                .colors
                .iter()
                .filter(|c| c.color == [1, 2, 3, 0])
                .count(),
            2
        );
        let next = s.create("Second").unwrap();
        let d = s.add([9; 4]).unwrap();
        s.drop_before(copy, d, false).unwrap();
        assert_eq!(s.tables[1].colors[0].id, copy);
        assert_eq!(s.selected, Some(next));
        s.tables[1].layout = Layout {
            size: 40,
            columns: Some(7),
            scroll: false,
        };
        let saved = serde_json::to_vec(&s).unwrap();
        assert_eq!(serde_json::from_slice::<Settings>(&saved).unwrap(), s);
        let before = s.clone();
        assert!(s.drop_before(a, u64::MAX, false).is_err());
        assert_eq!(s, before);
        assert!(s.edit_color(copy, " ", [3; 4]).is_err());
        assert_eq!(s, before);
        assert!(s.drop_before(u64::MAX, d, true).is_err());
        assert_eq!(s, before);
        let l = Layout::default();
        assert_eq!(l.columns_for(10.0), 1);
        assert_eq!(l.columns_for(260.0), 10);
        assert_eq!(s.tables[1].layout.columns_for(100.0), 7);
        assert_eq!(s.tables[1].layout.columns_for(1000.0), 7);
        s.delete_color(d).unwrap();
        s.delete_color(copy).unwrap();
        assert!(s.tables[1].colors.is_empty());
        assert!(s.valid());
    }
    #[test]
    fn copy_capacity_and_own_undo_are_atomic() {
        let mut s = Settings::default();
        for n in 0..256 {
            s.add([n as u8, 1, 2, 128]).unwrap();
        }
        let before = s.clone();
        let a = s.tables[0].colors[0].id;
        let b = s.tables[0].colors[1].id;
        assert!(s.drop_before(a, b, true).is_err());
        assert_eq!(s, before);
        s.drop_before(a, b, false).unwrap();
        assert_eq!(s, before);
        let mut h = History::default();
        s.delete_color(a).unwrap();
        h.record(before.clone(), &s);
        let after = s.clone();
        assert!(h.undo(&mut s));
        assert_eq!(s, before);
        assert!(h.redo(&mut s));
        assert_eq!(s, after);
    }
}
