use super::*;
use crate::{document::DEFAULT_DOCUMENT_BUDGET, surface_effects};
impl Renderer<'_> {
    pub(super) fn pre_effect_alpha(
        &self,
        node: Node,
        point: [i64; 2],
        original: Point,
        depth: usize,
    ) -> Result<f64, String> {
        let renderer = Renderer {
            document: self.document,
            stroke: self.stroke,
            plan: PlanRef::Borrowed(self.plan()),
            sampling: Sampling {
                pre_effects: true,
                ..Default::default()
            },
        };
        Ok(renderer.source(node, point, original, depth)?.alpha())
    }
    pub(super) fn surface_contribution(&self, n: usize, point: [i64; 2]) -> Result<Sample, String> {
        self.plan()
            .surfaces
            .get(&self.document.scene.layer_ids.id(n))
            .ok_or("Missing surface cache")?
            .sample(point)
            .coverage(p::factor(self.document, p::raster(self.document, n)))
    }
    pub(super) fn build_surface_caches(
        &mut self,
        tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        let d = self.document;
        let mut caches = std::collections::BTreeMap::new();
        let mut used = 0usize;
        // Build every source before installing any generated cache: a clip base can never
        // read another effect's expanded alpha or recursively depend on its output.
        for (&id, stack) in &d.scene.surface_effects.rasters {
            let n = crate::layers::index(d, id).ok_or("Missing surface-effect source layer")?;
            if !stack.active() {
                continue;
            }
            let extent = surface_effects::source_extent(d, n);
            let style = d
                .scene
                .appearance
                .get(p::raster(d, n))
                .filter(|s| s.enabled);
            let style = style.map(|s| {
                let mut s = s.clone();
                s.scope_space = s.scope_space.then(d.scene.transform);
                s
            });
            let pending = surface_effects::workspace_bytes(extent, stack)?;
            let appearance = self
                .plan()
                .caches
                .values()
                .map(|c| c.bytes())
                .sum::<usize>();
            if d.resident_bytes()
                .saturating_add(
                    self.stroke
                        .map_or(0, |s| s.changed_pixels().saturating_mul(256)),
                )
                .saturating_add(appearance)
                .saturating_add(used)
                .saturating_add(pending)
                > DEFAULT_DOCUMENT_BUDGET
            {
                return Err("Surface-effect renderer exceeds resident memory budget".into());
            }
            let base = d.scene.clipping.binding(p::raster(d, n));
            let image = surface_effects::render_with_appearance(
                extent,
                stack,
                pending,
                |point| {
                    let original = d
                        .scene
                        .transform
                        .inverse()
                        .map(point.map(|v| v as f64 + 0.5));
                    let target = p::raster(d, n);
                    let raw = self.expression(self.raster_raw(n, point, original)?, target)?;
                    let raw = if let Some(style) =
                        d.scene.appearance.get(target).filter(|s| s.enabled)
                        && let Some(color) = style.color
                    {
                        self.plan().space.mix(
                            raw,
                            raw.with_rgb(color.map(|v| v as f64 / 255.))?,
                            style.weight(original),
                            crate::color_mix::Space::Standard,
                            0.,
                        )?
                    } else {
                        raw
                    };
                    let mut value = self
                        .shade(self.paper(Node::Raster(n)), raw, original)?
                        .coverage(d.scene.layer_effects.mask(target, original) as f64)?;
                    if let Some(base) = base {
                        if !crate::clipping::active(d, p::raster(d, n)) {
                            return Ok(Sample::default());
                        }
                        let base = crate::clipping::node(d, base).ok_or("Missing clipping base")?;
                        value = value.coverage(self.source(base, point, original, 0)?.alpha())?;
                    }
                    self.plan().space.to_srgb(value)
                },
                style.as_ref(),
                &mut *tick,
            )?;
            used = used.saturating_add(image.pixels.len() * 16);
            // Cache in the working profile. Kernels operate in explicitly encoded sRGB.
            let mut image = image;
            for (i, pixel) in image.pixels.iter_mut().enumerate() {
                if i % 128 == 0 {
                    tick(
                        i,
                        image.extent.width as usize * image.extent.height as usize,
                    )?;
                }
                *pixel = self
                    .plan()
                    .space
                    .from_srgb(Sample::new(pixel.map(f64::from))?)?
                    .components()
                    .map(|v| v as f32);
            }
            caches.insert(id, image);
        }
        match &mut self.plan {
            PlanRef::Owned(p) => p.surfaces = caches,
            PlanRef::Borrowed(_) => return Err("Immutable surface render plan".into()),
        }
        Ok(())
    }
    /// Render this layer's final effect output before its opacity and parent-folder masks.
    pub fn surface_sample(&self, layer: usize, point: [i64; 2]) -> Result<Sample, String> {
        let id = self.document.scene.layer_ids.id(layer);
        if let Some(image) = self.plan().surfaces.get(&id) {
            Ok(image.sample(point))
        } else {
            self.intrinsic_raster(layer, point)
        }
    }
}
