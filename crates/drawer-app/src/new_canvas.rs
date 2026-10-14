use drawer_core::document::{Background, CanvasSpec, CreateError};

pub struct NewCanvas {
    pub width: String,
    pub height: String,
    pub dpi: String,
    pub transparent: bool,
    pub color: [u8; 3],
    pub error: Option<String>,
}
impl Default for NewCanvas {
    fn default() -> Self {
        Self {
            width: "1200".into(),
            height: "900".into(),
            dpi: "300".into(),
            transparent: false,
            color: [255; 3],
            error: None,
        }
    }
}
impl NewCanvas {
    pub fn spec(&self) -> Result<CanvasSpec, CreateError> {
        Ok(CanvasSpec {
            width: self
                .width
                .trim()
                .parse()
                .map_err(|_| CreateError::InvalidDimensions)?,
            height: self
                .height
                .trim()
                .parse()
                .map_err(|_| CreateError::InvalidDimensions)?,
            dpi: self
                .dpi
                .trim()
                .parse()
                .map_err(|_| CreateError::InvalidDpi)?,
            background: if self.transparent {
                Background::Transparent
            } else {
                Background::Color(self.color)
            },
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_text_never_turns_into_a_valid_canvas() {
        for width in ["", "-1", "1.5", "NaN", "4294967296"] {
            let draft = NewCanvas {
                width: width.into(),
                ..Default::default()
            };
            assert_eq!(draft.spec().unwrap_err(), CreateError::InvalidDimensions);
        }
        let draft = NewCanvas {
            width: " 1 ".into(),
            height: "1".into(),
            ..Default::default()
        };
        assert_eq!(draft.spec().unwrap().width, 1);
    }
}
