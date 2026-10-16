//! Owned, bounded normalized fibre geometry; no vendor brush data is consumed.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fibre {
    pub position: [f64; 2],
    pub thickness: f64,
    pub length: f64,
    pub angle: f64,
    pub density: f64,
}
impl Default for Fibre {
    fn default() -> Self {
        Self {
            position: [0.0; 2],
            thickness: 0.1,
            length: 1.0,
            angle: 0.0,
            density: 1.0,
        }
    }
}
impl Fibre {
    fn valid(self) -> bool {
        self.position
            .iter()
            .all(|x| x.is_finite() && x.abs() <= 1.0)
            && self.thickness.is_finite()
            && (0.0..=2.0).contains(&self.thickness)
            && self.length.is_finite()
            && (0.0..=4.0).contains(&self.length)
            && self.angle.is_finite()
            && (-180.0..=180.0).contains(&self.angle)
            && self.density.is_finite()
            && (0.0..=1.0).contains(&self.density)
    }
}
fn items<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Fibre>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<Fibre>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 128 fibres")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut v = Vec::new();
            while let Some(item) = seq.next_element()? {
                if v.len() == 128 {
                    return Err(serde::de::Error::custom("Fibre limit"));
                }
                v.push(item);
            }
            Ok(v)
        }
    }
    d.deserialize_seq(Visitor)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    pub version: u32,
    #[serde(deserialize_with = "items")]
    pub fibres: Vec<Fibre>,
}
impl Default for Set {
    fn default() -> Self {
        Self {
            version: 1,
            fibres: vec![],
        }
    }
}
impl Set {
    pub fn valid(&self) -> bool {
        self.version == 1 && self.fibres.len() <= 128 && self.fibres.iter().all(|f| f.valid())
    }
    /// Round-capped segments; zero width/density produces no ink, zero length a dot.
    pub fn sample(&self, q: [f64; 2], resolution: u32) -> f64 {
        if q.iter().any(|x| !x.is_finite() || x.abs() > 1.0) {
            return 0.0;
        }
        self.fibres
            .iter()
            .filter(|f| f.thickness > 0.0 && f.density > 0.0)
            .map(|f| {
                let (s, c) = f.angle.to_radians().sin_cos();
                let d = [q[0] - f.position[0], q[1] - f.position[1]];
                let along = (d[0] * c + d[1] * s).clamp(-f.length * 0.5, f.length * 0.5);
                let distance = (d[0] - along * c).hypot(d[1] - along * s);
                ((f.thickness * 0.5 - distance) * resolution as f64 * 0.5 + 0.5).clamp(0.0, 1.0)
                    * f.density
            })
            .fold(0.0, f64::max)
    }
    pub fn read(path: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("Expected a fibre JSON file"));
        }
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        let set: Self = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if bytes.len() > 65536 || !set.valid() {
            return Err(std::io::Error::other("Invalid fibre set / 64 KiB limit"));
        }
        Ok(set)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_and_bounds() {
        let mut s = Set::default();
        assert!(s.valid());
        assert_eq!(s.sample([0.0; 2], 128), 0.0);
        s.fibres.push(Fibre::default());
        assert_eq!(s.sample([0.0; 2], 128), 1.0);
        assert_eq!(s.sample([0.0, 0.5], 128), 0.0);
        s.fibres[0].angle = 90.0;
        assert_eq!(s.sample([0.0, 0.4], 128), 1.0);
        s.fibres[0].length = 0.0;
        assert_eq!(s.sample([0.0, 0.4], 128), 0.0);
        assert_eq!(s.sample([0.0; 2], 128), 1.0);
        s.fibres[0].thickness = 0.0;
        assert_eq!(s.sample([0.0; 2], 128), 0.0);
        s.fibres = vec![Fibre::default(); 128];
        assert!(s.valid());
        s.fibres.push(Fibre::default());
        assert!(!s.valid());
        assert!(serde_json::from_slice::<Set>(&serde_json::to_vec(&s).unwrap()).is_err());
        s.fibres = vec![Fibre {
            length: 4.01,
            ..Default::default()
        }];
        assert!(!s.valid());
        s.fibres[0].length = f64::NAN;
        assert!(!s.valid());
    }
    #[test]
    fn file_limit_version_and_zero_density() {
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("fibres.json");
        let mut s = Set {
            version: 1,
            fibres: vec![Fibre {
                density: 0.0,
                ..Default::default()
            }],
        };
        assert_eq!(s.sample([0.0; 2], 128), 0.0);
        std::fs::write(&path, serde_json::to_vec(&s).unwrap()).unwrap();
        assert!(Set::read(&path).unwrap() == s);
        s.version = 2;
        std::fs::write(&path, serde_json::to_vec(&s).unwrap()).unwrap();
        assert!(Set::read(&path).is_err());
        std::fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Set::read(&path).is_err());
    }
}
