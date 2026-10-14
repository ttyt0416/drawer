use drawer_core::document::{Background, CanvasSpec, CreateError};

pub struct NewCanvas {
    pub size: drawer_core::canvas_size::Dimensions,
    pub transparent: bool,
    pub color: [u8; 3],
    pub error: Option<String>,
}
impl Default for NewCanvas {
    fn default() -> Self {
        Self {
            size: drawer_core::canvas_size::Dimensions::new(1200, 900, 300.0),
            transparent: false,
            color: [255; 3],
            error: None,
        }
    }
}
impl NewCanvas {
    pub fn spec(&self) -> Result<CanvasSpec, CreateError> {
        let dpi =
            drawer_core::canvas_size::dpi(&self.size.dpi).map_err(|_| CreateError::InvalidDpi)?;
        let [width, height] = self
            .size
            .pixels()
            .map_err(|_| CreateError::InvalidDimensions)?;
        Ok(CanvasSpec {
            width,
            height,
            dpi,
            background: if self.transparent {
                Background::Transparent
            } else {
                Background::Color(self.color)
            },
        })
    }
}
impl std::ops::Deref for NewCanvas {
    type Target = drawer_core::canvas_size::Dimensions;
    fn deref(&self) -> &Self::Target {
        &self.size
    }
}
impl std::ops::DerefMut for NewCanvas {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.size
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_text_never_turns_into_a_valid_canvas() {
        for width in ["", "-1", "1.5", "NaN", "4294967296"] {
            let mut size = drawer_core::canvas_size::Dimensions::new(1200, 900, 300.0);
            size.width = width.into();
            let draft = NewCanvas {
                size,
                ..Default::default()
            };
            assert_eq!(draft.spec().unwrap_err(), CreateError::InvalidDimensions);
        }
        let mut size = drawer_core::canvas_size::Dimensions::new(1, 1, 300.0);
        size.width = " 1 ".into();
        let draft = NewCanvas {
            size,
            ..Default::default()
        };
        assert_eq!(draft.spec().unwrap().width, 1);
    }
}
